package segmentlog

// The pooled client (issue 134): many goroutines share a few pipelined connections (ADR 0063).

import (
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
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
// When 63 more requests, up to the bound, are queued behind it, each expecting the revision the
// one before it gets
// Then they go out in one more write, and the answers return in request order
func TestPoolCoalescesQueuedRequests(t *testing.T) {
	d := newLogDir(t)
	d.start(t, "")
	c := dial(t, d)
	conn := &gatedConn{ReadWriteCloser: c.conn, gate: make(chan struct{})}
	c.conn = conn
	p := newPool(d.network, d.address, []*Client{c})
	t.Cleanup(func() { p.Close() })

	const n = maxInFlight
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

// TRANSPORT-UNKNOWN and TRANSPORT-RESTART through a pool.
//
// Given a pool of 4 connections and a server whose linger holds every group
// When 256 goroutines each send a keyed append, filling every connection's 64 places, and the
// server dies before any answer
// Then every append gets ErrUnknown, never a rejection
// When a request is sent while the server is down
// Then it gets ErrNotSent
// When the server restarts on the same directory
// Then the same pool redials, and a fully keyed retry through it resolves each unknown outcome
func TestPoolUnknownOnClose(t *testing.T) {
	d := newLogDir(t)
	cmd := d.start(t, "", "--linger-us", "5000000")
	p := dialPool(t, d, 4)
	const callers = 4 * maxInFlight
	submits := make([]submit, callers)
	errs := make([]error, callers)
	var wg sync.WaitGroup
	for g := range callers {
		submits[g] = submit{records: []NewRecord{keyed(fmt.Sprint("s", g), fmt.Sprint("k", g), empty, []byte("p"))}}
		wg.Go(func() { _, errs[g] = p.Do(Append(submits[g].records)) })
	}
	// Every request is written before the kill, so none of them can be ErrNotSent.
	for _, s := range p.slots {
		waitFor(t, func() bool {
			s.conn.mu.Lock()
			defer s.conn.mu.Unlock()
			return len(s.conn.waiting) == maxInFlight && s.conn.unsent == 0
		})
	}
	cmd.Process.Kill()
	cmd.Wait()
	wg.Wait()
	for g, err := range errs {
		if !errors.Is(err, ErrUnknown) {
			t.Fatalf("append %d after the server died: %v, want ErrUnknown", g, err)
		}
	}
	if _, err := p.Do(Watermark()); !errors.Is(err, ErrNotSent) {
		t.Fatalf("request while the server is down: %v, want ErrNotSent", err)
	}
	d.start(t, "")
	for g, s := range submits {
		a, err := p.Do(Append(s.records))
		if err != nil || a.Status != 0 {
			t.Fatalf("retry of append %d through the redialed pool: %+v %v", g, a, err)
		}
		submits[g].positions = a.Positions
	}
	resolve(t, dial(t, d), submits)
}

// Given a pool of 2 connections
// When it is closed
// Then a later request gets ErrPoolClosed
func TestPoolClosed(t *testing.T) {
	d := newLogDir(t)
	d.start(t, "")
	p := dialPool(t, d, 2)
	p.Close()
	for range 2 {
		if _, err := p.Do(Watermark()); !errors.Is(err, ErrPoolClosed) {
			t.Fatalf("request on a closed pool: %v, want ErrPoolClosed", err)
		}
	}
}

// Given a pool of 1 connection whose first write is held, with 64 requests queued
// When a 65th request is sent
// Then it is not queued while 64 wait for answers
// When the held write is released
// Then all 65 are answered in order
func TestPoolBoundsRequestsInFlight(t *testing.T) {
	d := newLogDir(t)
	d.start(t, "")
	c := dial(t, d)
	conn := &gatedConn{ReadWriteCloser: c.conn, gate: make(chan struct{})}
	c.conn = conn
	p := newPool(d.network, d.address, []*Client{c})
	t.Cleanup(func() { p.Close() })

	replies := make([]<-chan poolResult, maxInFlight+1)
	for i := range maxInFlight {
		replies[i] = p.send(Watermark())
	}
	last := make(chan (<-chan poolResult))
	go func() { last <- p.send(Watermark()) }()
	// A negative check: the 65th has had time to queue, and must not have.
	time.Sleep(50 * time.Millisecond)
	pc := p.slots[0].conn
	pc.mu.Lock()
	queued := len(pc.waiting)
	pc.mu.Unlock()
	if queued != maxInFlight {
		t.Fatalf("%d requests queued, want %d", queued, maxInFlight)
	}
	close(conn.gate)
	replies[maxInFlight] = <-last
	for i, reply := range replies {
		if res := <-reply; res.err != nil || res.answer.Status != 0 {
			t.Fatalf("answer %d: %+v %v", i, res.answer, res.err)
		}
	}
}

func waitFor(t *testing.T, cond func() bool) {
	t.Helper()
	for deadline := time.Now().Add(10 * time.Second); !cond(); time.Sleep(time.Millisecond) {
		if time.Now().After(deadline) {
			t.Fatal("condition not reached in 10 s")
		}
	}
}

// fakeServer listens on a Unix socket and hands each accepted connection to serve.
func fakeServer(t *testing.T, serve func(net.Conn)) string {
	t.Helper()
	dir, err := os.MkdirTemp("/tmp", "sl-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	socket := filepath.Join(dir, "fake")
	l, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { l.Close() })
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				return
			}
			go serve(c)
		}
	}()
	return socket
}

// failedPool is a pool of 1 connection to d whose connection has failed, and whose redials go to
// address.
func failedPool(t *testing.T, d logDir, address string) *Pool {
	t.Helper()
	p := newPool("unix", address, []*Client{dial(t, d)})
	t.Cleanup(func() { p.Close() })
	p.slots[0].conn.fail(ErrNotSent)
	return p
}

// Given a pool whose connection has failed, and a server that accepts and closes during Hello
// When a request redials
// Then it gets ErrNotSent, never ErrUnknown: the request was never written
func TestPoolRedialCutShortIsNotSent(t *testing.T) {
	d := newLogDir(t)
	d.start(t, "")
	p := failedPool(t, d, fakeServer(t, func(c net.Conn) { c.Close() }))
	_, err := p.Do(Watermark())
	if !errors.Is(err, ErrNotSent) || errors.Is(err, ErrUnknown) {
		t.Fatalf("redial cut short: %v, want ErrNotSent and not ErrUnknown", err)
	}
}

// Given a pool whose connection has failed, and a server that accepts and never answers Hello
// When a request redials and hangs in Hello
// Then Close still returns
// When the server finally closes the connection
// Then the request gets ErrPoolClosed
func TestPoolCloseDoesNotWaitForRedial(t *testing.T) {
	d := newLogDir(t)
	d.start(t, "")
	accepted := make(chan net.Conn, 1)
	p := failedPool(t, d, fakeServer(t, func(c net.Conn) { accepted <- c }))
	res := make(chan error, 1)
	go func() { _, err := p.Do(Watermark()); res <- err }()
	hung := <-accepted
	closed := make(chan struct{})
	go func() { p.Close(); close(closed) }()
	select {
	case <-closed:
	case <-time.After(10 * time.Second):
		t.Fatal("Close waited for a redial")
	}
	hung.Close()
	if err := <-res; !errors.Is(err, ErrPoolClosed) {
		t.Fatalf("request whose redial outlived Close: %v, want ErrPoolClosed", err)
	}
}

// Given a pool of 1 connection whose first write is held, with 10 requests queued behind it
// When the pool is closed
// Then the request being written gets ErrUnknown, and the 10 never written get ErrPoolClosed
func TestPoolCloseRefusesUnwrittenRequests(t *testing.T) {
	d := newLogDir(t)
	d.start(t, "")
	c := dial(t, d)
	conn := &gatedConn{ReadWriteCloser: c.conn, gate: make(chan struct{})}
	c.conn = conn
	t.Cleanup(func() { close(conn.gate) })
	p := newPool(d.network, d.address, []*Client{c})
	first := p.send(Watermark())
	waitFor(t, func() bool { return conn.writes.Load() == 1 })
	var queued []<-chan poolResult
	for range 10 {
		queued = append(queued, p.send(Watermark()))
	}
	p.Close()
	if res := <-first; !errors.Is(res.err, ErrUnknown) {
		t.Fatalf("request being written: %v, want ErrUnknown", res.err)
	}
	for i, reply := range queued {
		if res := <-reply; !errors.Is(res.err, ErrPoolClosed) {
			t.Fatalf("unwritten request %d: %v, want ErrPoolClosed", i, res.err)
		}
	}
}

// Given a pool of 1 connection
// When a caller sends Hello through it
// Then the pool refuses it without sending, and the connection still serves the next request
func TestPoolRefusesHello(t *testing.T) {
	d := newLogDir(t)
	d.start(t, "")
	p := dialPool(t, d, 1)
	if _, err := p.Do(Hello(MinVersion, MaxVersion)); err == nil || errors.Is(err, ErrUnknown) {
		t.Fatalf("Hello through a pool: %v, want a refusal", err)
	}
	if a, err := p.Do(Watermark()); err != nil || a.Status != 0 {
		t.Fatalf("request after the refused Hello: %+v %v", a, err)
	}
}
