package segmentlog

// The pooled client (issue 134): many goroutines share a few pipelined connections (ADR 0063).

import (
	"errors"
	"fmt"
	"io"
	"slices"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func dialPool(t *testing.T, d logDir, n int) *Pool {
	t.Helper()
	p, err := DialPool(d.network, d.address, n)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { p.Close() })
	return p
}

// Given a pool of 4 connections
// When 1,000 goroutines each append 3 keyed records to their own stream, one at a time
// Then every append succeeds at the next revision of its stream, no two records share an offset,
// and each record is in the log at the position it got, once
func TestPoolSharedByGoroutines(t *testing.T) {
	d := newLogDir(t)
	d.start(t, "")
	p := dialPool(t, d, 4)
	const callers, each = 1000, 3
	submits := make([]submit, callers*each)
	var wg sync.WaitGroup
	errs := make(chan error, callers)
	for g := range callers {
		wg.Go(func() {
			stream := fmt.Sprint("s", g)
			for i := range each {
				expected := Expected{Revision: uint64(i - 1)}
				if i == 0 {
					expected = empty
				}
				s := submit{records: []NewRecord{keyed(stream, fmt.Sprint(stream, "/", i), expected, []byte("p"))}}
				a, err := p.Do(Append(s.records))
				if err != nil || a.Status != 0 || a.Positions[0].Revision != uint64(i) {
					errs <- fmt.Errorf("%s append %d: %+v %v", stream, i, a, err)
					return
				}
				s.positions = a.Positions
				submits[g*each+i] = s
			}
		})
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Fatal(err)
	}
	offsets := map[uint64]bool{}
	for _, s := range submits {
		if offsets[s.positions[0].Offset] {
			t.Fatalf("offset %d given twice", s.positions[0].Offset)
		}
		offsets[s.positions[0].Offset] = true
	}
	resolve(t, dial(t, d), submits)
}

// gatedConn counts writes and holds the first one until gate closes.
type gatedConn struct {
	io.ReadWriteCloser
	writes atomic.Int32
	gate   chan struct{}
}

func (g *gatedConn) Write(b []byte) (int, error) {
	if g.writes.Add(1) == 1 {
		<-g.gate
	}
	return g.ReadWriteCloser.Write(b)
}

// Given a pool of 1 connection whose first write is held
// When 200 more requests are queued behind it, each expecting the revision the one before it gets
// Then they go out in one more write, and the answers return in request order
func TestPoolCoalescesQueuedRequests(t *testing.T) {
	d := newLogDir(t)
	d.start(t, "")
	c := dial(t, d)
	conn := &gatedConn{ReadWriteCloser: c.conn, gate: make(chan struct{})}
	c.conn = conn
	p := newPool([]*Client{c})
	t.Cleanup(func() { p.Close() })

	const n = 201
	replies := make([]<-chan poolResult, n)
	for i := range n {
		expected := Expected{Revision: uint64(i - 1)}
		if i == 0 {
			expected = empty
		}
		replies[i] = p.send(Append([]NewRecord{{Stream: []byte("s"), Expected: expected, Payload: []byte("p")}}))
		if i == 0 {
			for conn.writes.Load() == 0 {
				time.Sleep(time.Millisecond)
			}
		}
	}
	close(conn.gate)
	for i, reply := range replies {
		res := <-reply
		want := []Position{{uint64(i), uint64(i)}}
		if res.err != nil || res.answer.Status != 0 || !slices.Equal(res.answer.Positions, want) {
			t.Fatalf("answer %d: %+v %v, want %v", i, res.answer, res.err, want)
		}
	}
	if w := conn.writes.Load(); w != 2 {
		t.Fatalf("%d writes, want 2", w)
	}
}

// TRANSPORT-UNKNOWN through a pool.
//
// Given a pool of 4 connections and a server whose linger holds every group
// When 20 goroutines each send a keyed append, and the server dies before any answer
// Then every append gets ErrUnknown, never a rejection, and a later request gets ErrPoolClosed
// When the server restarts on the same directory
// Then a fully keyed retry resolves each unknown outcome
func TestPoolUnknownOnClose(t *testing.T) {
	d := newLogDir(t)
	cmd := d.start(t, "", "--linger-us", "5000000")
	p := dialPool(t, d, 4)
	const callers = 20
	submits := make([]submit, callers)
	errs := make([]error, callers)
	var wg sync.WaitGroup
	for g := range callers {
		submits[g] = submit{records: []NewRecord{keyed(fmt.Sprint("s", g), fmt.Sprint("k", g), empty, []byte("p"))}}
		wg.Go(func() { _, errs[g] = p.Do(Append(submits[g].records)) })
	}
	time.Sleep(100 * time.Millisecond)
	cmd.Process.Kill()
	cmd.Wait()
	wg.Wait()
	for g, err := range errs {
		if !errors.Is(err, ErrUnknown) {
			t.Fatalf("append %d after the server died: %v, want ErrUnknown", g, err)
		}
	}
	if _, err := p.Do(Watermark()); !errors.Is(err, ErrPoolClosed) {
		t.Fatalf("request on a failed pool: %v, want ErrPoolClosed", err)
	}
	d.start(t, "")
	resolve(t, dial(t, d), submits)
}
