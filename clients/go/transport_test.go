package segmentlog

// TRANSPORT-UNKNOWN, TRANSPORT-RESTART and the poison exit (testing.md, ADR 0058). Each one kills
// or loses a server process, so these run as code, not as conformance data.

import (
	"bytes"
	"errors"
	"fmt"
	"os/exec"
	"slices"
	"testing"
	"time"
)

// submit is one fully keyed append and what is known about its outcome.
type submit struct {
	records   []NewRecord
	positions []Position // nil while the outcome is unknown
}

func keyed(stream, key string, expected Expected, payload []byte) NewRecord {
	return NewRecord{Stream: []byte(stream), Expected: expected, Key: []byte(key), Payload: payload}
}

var empty = Expected{Empty: true}

func dial(t *testing.T, d logDir) *Client {
	t.Helper()
	c, err := Dial("unix", d.socket)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	return c
}

func positions(a Answer) []Position { return a.Positions }

// resolve replays every submit with its original keys, streams, payloads and expectations, and
// checks oracle check 4: an acknowledged submit returns its original positions with
// deduplicated set, and an unknown one either returns its survivors or appends anew. Then it
// reads the whole log and checks that each record appears once, at the position its replay gave.
func resolve(t *testing.T, c *Client, submits []submit) {
	t.Helper()
	want := map[string]Position{}
	count := 0
	for i, s := range submits {
		a, err := c.Do(Append(s.records))
		if err != nil || a.Status != 0 {
			t.Fatalf("replay of submit %d: %v %v", i, a.Status, err)
		}
		if s.positions != nil && (!a.Deduplicated || !slices.Equal(a.Positions, s.positions)) {
			t.Fatalf("acknowledged submit %d replayed as %+v, want %v deduplicated", i, a, s.positions)
		}
		for j, r := range s.records {
			want[string(r.Key)] = a.Positions[j]
		}
		count += len(s.records)
	}
	var records []Record
	for {
		a, err := c.Do(Read(uint64(len(records)), 1000))
		if err != nil || a.Status != 0 {
			t.Fatalf("read: %v %v", a.Status, err)
		}
		if len(a.Records) == 0 {
			break
		}
		records = append(records, a.Records...)
	}
	if len(records) != count {
		t.Fatalf("the log holds %d records, want %d", len(records), count)
	}
	for _, r := range records {
		p, ok := want[string(r.Key)]
		if !ok || p != (Position{r.Offset, r.Revision}) {
			t.Fatalf("record %+v, want position %+v", r, p)
		}
	}
}

// TRANSPORT-UNKNOWN: a connection that closes after a request and before its answer is an
// unknown outcome, never a rejection.
func TestTransportUnknown(t *testing.T) {
	d := newLogDir(t)
	// A long linger holds the group, so the kill lands before any answer.
	cmd := d.start(t, "", "--linger-us", "5000000")
	c := dial(t, d)
	s := submit{records: []NewRecord{keyed("s", "k", empty, []byte("p"))}}
	if err := c.Send(Append(s.records)); err != nil {
		t.Fatal(err)
	}
	time.Sleep(100 * time.Millisecond)
	cmd.Process.Kill()
	cmd.Wait()
	if _, err := c.Receive(); !errors.Is(err, ErrUnknown) {
		t.Fatalf("receive after the server died: %v, want ErrUnknown", err)
	}
	d.start(t, "")
	resolve(t, dial(t, d), []submit{s})
}

// TRANSPORT-RESTART: after SIGKILL and a restart on the same directory, the client resolves its
// fully keyed unknown outcomes by retry.
func TestTransportRestart(t *testing.T) {
	d := newLogDir(t)
	cmd := d.start(t, "")
	c := dial(t, d)
	var submits []submit
	for i := range 300 {
		payload := []byte(fmt.Sprint("payload ", i))
		if i%3 == 2 {
			stream := fmt.Sprint("m", i)
			submits = append(submits, submit{records: []NewRecord{
				keyed(stream, fmt.Sprint("k", i, "a"), empty, payload),
				keyed(stream, fmt.Sprint("k", i, "b"), Expected{Revision: 0}, payload),
			}})
		} else {
			submits = append(submits, submit{records: []NewRecord{keyed(fmt.Sprint("s", i), fmt.Sprint("k", i), empty, payload)}})
		}
	}
	go func() {
		for _, s := range submits {
			if c.Send(Append(s.records)) != nil {
				return // the server is gone; the rest stay unknown
			}
		}
	}()
	for i := range submits {
		if i == 100 {
			cmd.Process.Kill()
			cmd.Wait()
		}
		a, err := c.Receive()
		if errors.Is(err, ErrUnknown) {
			break
		}
		if err != nil || a.Status != 0 {
			t.Fatalf("submit %d: %v %v", i, a.Status, err)
		}
		submits[i].positions = a.Positions
	}
	known := 0
	for _, s := range submits {
		if s.positions != nil {
			known++
		}
	}
	if known < 100 || known == len(submits) {
		t.Fatalf("%d of %d outcomes known; the kill should land mid-stream", known, len(submits))
	}
	d.start(t, "")
	resolve(t, dial(t, d), submits)
}

// A poisoned writer answers the request in flight, then the server exits with status 2. A
// restart on the same directory recovers, and the keyed retry resolves.
func TestPoisonExit(t *testing.T) {
	d := newLogDir(t)
	// The core never preallocates a segment, so a small file-size limit fails a later write with
	// EFBIG. Ignoring SIGXFSZ, which the server inherits, turns the signal into that error.
	cmd := d.start(t, `ulimit -f 16; trap "" XFSZ`)
	c := dial(t, d)
	payload := bytes.Repeat([]byte("x"), 1000)
	var submits []submit
	var failed Answer
	for i := range 100 {
		s := submit{records: []NewRecord{keyed(fmt.Sprint("s", i), fmt.Sprint("k", i), empty, payload)}}
		a, err := c.Do(Append(s.records))
		if err != nil {
			t.Fatalf("submit %d: %v", i, err)
		}
		if a.Status == 0 {
			s.positions = a.Positions
		}
		submits = append(submits, s)
		if a.Status != 0 {
			failed = a
			break
		}
	}
	if failed.Status.String() != "Indeterminate" {
		t.Fatalf("first failed append: %v, want Indeterminate", failed.Status)
	}
	if !c.Closed() {
		t.Fatal("the server kept the connection open after the writer was poisoned")
	}
	err := cmd.Wait()
	var exit *exec.ExitError
	if !errors.As(err, &exit) || exit.ExitCode() != 2 {
		t.Fatalf("server exit: %v, want status 2", err)
	}
	d.start(t, "")
	resolve(t, dial(t, d), submits)
}
