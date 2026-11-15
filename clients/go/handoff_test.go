package segmentlog

// HANDOFF (testing.md, ADR 0033, ADR 0060): acknowledged records outlive the writer and are read
// back by a read-only server. Over SEGMENT_LOG_TRANSPORT=firecracker each start is a new guest
// on the same volume, so the writer's guest is destroyed before the reader's boots.

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// appendAll appends n single-record submits and returns their acknowledged positions by key.
func appendAll(t *testing.T, c *Client, n int) map[string]Position {
	t.Helper()
	acked := map[string]Position{}
	for i := range n {
		key, expected := fmt.Sprint("k", i), empty
		if i >= 7 {
			expected = Expected{Revision: uint64(i/7 - 1)}
		}
		a, err := c.Do(Append([]NewRecord{keyed(fmt.Sprint("s", i%7), key, expected, []byte(key))}))
		if err != nil || a.Status != 0 {
			t.Fatalf("append %d: %v %v", i, a.Status, err)
		}
		acked[key] = a.Positions[0]
	}
	return acked
}

// readsBack checks that the log holds exactly the acknowledged records, at their positions.
func readsBack(t *testing.T, c *Client, acked map[string]Position) {
	t.Helper()
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
	if len(records) != len(acked) {
		t.Fatalf("read %d records, want %d", len(records), len(acked))
	}
	for _, r := range records {
		if p, ok := acked[string(r.Key)]; !ok || p != (Position{r.Offset, r.Revision}) || !bytes.Equal(r.Payload, r.Key) {
			t.Fatalf("record %+v, want position %+v", r, p)
		}
	}
}

// The reader answers like a sealed log: appends get Sealed, and Seal succeeds without stopping it.
func checkReadOnly(t *testing.T, d logDir, c *Client, records int) {
	t.Helper()
	if a, _ := c.Do(Append([]NewRecord{keyed("new", "new", empty, []byte("p"))})); a.Status.String() != "Sealed" {
		t.Fatalf("append to the reader: %v, want Sealed", a.Status)
	}
	if a, err := c.Do(Watermark()); err != nil || a.Status != 0 || a.Watermark != uint64(records) {
		t.Fatalf("watermark %+v %v, want %d", a, err, records)
	}
	if a, err := c.Do(Seal()); err != nil || a.Status != 0 {
		t.Fatalf("seal on the reader: %v %v", a.Status, err)
	}
	dial(t, d) // still serving
}

func TestHandoffSealed(t *testing.T) {
	// Given a writer that acknowledged records and sealed the log, then exited
	d := newLogDir(t)
	writer := d.start(t, "")
	acked := appendAll(t, dial(t, d), 50)
	if a, err := dial(t, d).Do(Seal()); err != nil || a.Status != 0 {
		t.Fatalf("seal: %v %v", a.Status, err)
	}
	if code := exitStatus(t, writer); code != 0 {
		t.Fatalf("writer exit %d after the seal, want 0", code)
	}

	// When a read-only server opens the surviving volume
	d.start(t, "", "--read-only")
	c := dial(t, d)

	// Then it reads every acknowledged record, and changes nothing
	readsBack(t, c, acked)
	checkReadOnly(t, d, c, len(acked))
}

func TestHandoffUnsealed(t *testing.T) {
	// Given a writer that acknowledged records and was killed without a seal (a crashed guest)
	d := newLogDir(t)
	writer := d.start(t, "")
	acked := appendAll(t, dial(t, d), 50)
	writer.Process.Kill()
	writer.Wait()

	// When a writable server recovers the log, and the deployment's policy finalizes it with a
	// seal (ADR 0033: sealing is the caller's decision, never implicit)
	recovery := d.start(t, "")
	recovered := dial(t, d)
	readsBack(t, recovered, acked)
	if a, err := recovered.Do(Seal()); err != nil || a.Status != 0 {
		t.Fatalf("seal after recovery: %v %v", a.Status, err)
	}
	// One writing kernel at a time (V5): the reader attaches only once the recovery has exited.
	if code := exitStatus(t, recovery); code != 0 {
		t.Fatalf("recovery exit %d after the seal, want 0", code)
	}

	// Then a read-only server reads every acknowledged record
	d.start(t, "", "--read-only")
	c := dial(t, d)
	readsBack(t, c, acked)
	checkReadOnly(t, d, c, len(acked))
}

// A read-only server needs a sealed log: on an unsealed one it exits 1 and says why, rather than
// recovering it (which writes).
func TestReadOnlyRefusesAnUnsealedLog(t *testing.T) {
	d := newLogDir(t)
	if os.Getenv("SEGMENT_LOG_TRANSPORT") == "firecracker" {
		t.Fatal("this case checks the server's exit status; run it over unix or vsock")
	}
	writer := d.start(t, "")
	appendAll(t, dial(t, d), 3)
	writer.Process.Kill()
	writer.Wait()

	args := append([]string{"--dir", filepath.Join(d.dir, "log"), "--read-only"}, d.listen...)
	var stderr bytes.Buffer
	cmd := exec.Command(os.Getenv("SEGMENT_LOG_SERVER"), args...)
	cmd.Stderr = &stderr
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	if code := exitStatus(t, cmd); code != 1 || !strings.Contains(stderr.String(), "NotSealed") {
		t.Fatalf("exit %d, stderr %q; want 1 and NotSealed", code, stderr.String())
	}
}
