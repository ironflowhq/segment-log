package segmentlog

// Runs the shared conformance cases in conformance/*.json against a server binary. The cases are
// data, so every transport runs the same ones (testing.md, M6 transport order and conformance).
//
// SEGMENT_LOG_SERVER names the binary: cargo build -p segment-log-server, then
// SEGMENT_LOG_SERVER=$PWD/target/debug/segment-log-server go test ./...

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"testing"
	"time"
)

type caseFile struct {
	Case       string   `json:"case"`
	Name       string   `json:"name"`
	ServerArgs []string `json:"server_args"`
	NoHello    bool     `json:"no_hello"`
	Steps      []step   `json:"steps"`
}

type step struct {
	Append []recordIn `json:"append"`
	Read   *struct {
		From uint64 `json:"from"`
		Max  uint32 `json:"max"`
	} `json:"read"`
	Watermark  *struct{} `json:"watermark"`
	StreamHead *string   `json:"stream_head"`
	Seal       *struct{} `json:"seal"`
	Raw        string    `json:"raw"`
	Pipeline   []step    `json:"pipeline"`
	Expect     *expect   `json:"expect"`
	Closed     bool      `json:"closed"`
}

type recordIn struct {
	Stream     string          `json:"stream"`
	Expected   json.RawMessage `json:"expected"`
	Key        *string         `json:"key"`
	Payload    string          `json:"payload"`
	PayloadLen int             `json:"payload_len"`
}

type recordOut struct {
	Offset   uint64  `json:"offset"`
	Revision uint64  `json:"revision"`
	Stream   string  `json:"stream"`
	Key      *string `json:"key"`
	Payload  string  `json:"payload"`
}

type expect struct {
	Status       string          `json:"status"`
	Positions    [][2]uint64     `json:"positions"`
	Deduplicated *bool           `json:"deduplicated"`
	Records      *[]recordOut    `json:"records"`
	Watermark    *uint64         `json:"watermark"`
	Head         *Head           `json:"head"`
	Absent       bool            `json:"absent"`
	Index        *uint32         `json:"index"`
	Actual       json.RawMessage `json:"actual"`
}

func (r recordIn) build() (NewRecord, error) {
	n := NewRecord{Stream: []byte(r.Stream), Payload: []byte(r.Payload)}
	if r.PayloadLen > 0 {
		n.Payload = bytes.Repeat([]byte("x"), r.PayloadLen)
	}
	if r.Key != nil {
		n.Key = []byte(*r.Key)
	}
	if string(r.Expected) == `"empty"` {
		n.Expected.Empty = true
	} else if err := json.Unmarshal(r.Expected, &n.Expected.Revision); err != nil {
		return n, fmt.Errorf("expected must be \"empty\" or a revision: %s", r.Expected)
	}
	return n, nil
}

func (s step) request() (Request, error) {
	switch {
	case s.Append != nil:
		records := make([]NewRecord, len(s.Append))
		for i, r := range s.Append {
			var err error
			if records[i], err = r.build(); err != nil {
				return nil, err
			}
		}
		return Append(records), nil
	case s.Read != nil:
		return Read(s.Read.From, s.Read.Max), nil
	case s.Watermark != nil:
		return Watermark(), nil
	case s.StreamHead != nil:
		return StreamHead([]byte(*s.StreamHead)), nil
	case s.Seal != nil:
		return Seal(), nil
	case s.Raw != "":
		return hex.DecodeString(s.Raw)
	}
	return nil, fmt.Errorf("step names no request")
}

func check(e *expect, a Answer) error {
	if e == nil {
		return fmt.Errorf("step has no expect")
	}
	if a.Status.String() != e.Status {
		return fmt.Errorf("status %v, want %s (answer %+v)", a.Status, e.Status, a)
	}
	if e.Positions != nil {
		got := make([][2]uint64, len(a.Positions))
		for i, p := range a.Positions {
			got[i] = [2]uint64{p.Offset, p.Revision}
		}
		if !slices.Equal(got, e.Positions) {
			return fmt.Errorf("positions %v, want %v", got, e.Positions)
		}
	}
	if e.Deduplicated != nil && a.Deduplicated != *e.Deduplicated {
		return fmt.Errorf("deduplicated %v, want %v", a.Deduplicated, *e.Deduplicated)
	}
	if e.Records != nil {
		got := make([]recordOut, len(a.Records))
		for i, r := range a.Records {
			got[i] = recordOut{r.Offset, r.Revision, string(r.Stream), nil, string(r.Payload)}
			if r.Key != nil {
				k := string(r.Key)
				got[i].Key = &k
			}
		}
		want, _ := json.Marshal(*e.Records)
		have, _ := json.Marshal(got)
		if !bytes.Equal(want, have) {
			return fmt.Errorf("records %s, want %s", have, want)
		}
	}
	if e.Watermark != nil && a.Watermark != *e.Watermark {
		return fmt.Errorf("watermark %d, want %d", a.Watermark, *e.Watermark)
	}
	if e.Head != nil && (a.Head == nil || *a.Head != *e.Head) {
		return fmt.Errorf("head %+v, want %+v", a.Head, *e.Head)
	}
	if e.Absent && a.Head != nil {
		return fmt.Errorf("head %+v, want absent", *a.Head)
	}
	if e.Index != nil && a.Index != *e.Index {
		return fmt.Errorf("index %d, want %d", a.Index, *e.Index)
	}
	if e.Actual != nil {
		got := "none"
		if a.Actual != nil {
			got = fmt.Sprint(*a.Actual)
		}
		want := string(bytes.Trim(e.Actual, `"`))
		if got != want {
			return fmt.Errorf("actual %s, want %s", got, want)
		}
	}
	return nil
}

func startServer(t *testing.T, args []string) string {
	t.Helper()
	bin := os.Getenv("SEGMENT_LOG_SERVER")
	if bin == "" {
		t.Fatal("set SEGMENT_LOG_SERVER to the server binary (cargo build -p segment-log-server)")
	}
	// Under /tmp, not t.TempDir: macOS limits a socket path to 104 bytes.
	dir, err := os.MkdirTemp("/tmp", "sl-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	socket := filepath.Join(dir, "s")
	var stderr bytes.Buffer
	cmd := exec.Command(bin, append([]string{"--dir", filepath.Join(dir, "log"), "--unix", socket}, args...)...)
	cmd.Stderr = &stderr
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		cmd.Process.Kill()
		cmd.Wait()
		if t.Failed() {
			t.Logf("server stderr:\n%s", stderr.String())
		}
	})
	for deadline := time.Now().Add(10 * time.Second); time.Now().Before(deadline); time.Sleep(10 * time.Millisecond) {
		if conn, err := net.Dial("unix", socket); err == nil {
			conn.Close()
			return socket
		}
	}
	t.Fatalf("server did not listen on %s; stderr:\n%s", socket, stderr.String())
	return ""
}

func TestConformance(t *testing.T) {
	files, err := filepath.Glob("../../conformance/*.json")
	if err != nil || len(files) == 0 {
		t.Fatalf("no conformance cases found: %v", err)
	}
	for _, file := range files {
		data, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		var c caseFile
		if err := json.Unmarshal(data, &c); err != nil {
			t.Fatalf("%s: %v", file, err)
		}
		t.Run(filepath.Base(file), func(t *testing.T) {
			socket := startServer(t, c.ServerArgs)
			dial := Dial
			if c.NoHello {
				dial = DialRaw
			}
			client, err := dial("unix", socket)
			if err != nil {
				t.Fatal(err)
			}
			defer client.Close()
			for i, s := range c.Steps {
				if err := run(client, s); err != nil {
					t.Fatalf("%s (%s) step %d: %v", c.Name, c.Case, i, err)
				}
			}
		})
	}
}

func run(c *Client, s step) error {
	steps := s.Pipeline
	if steps == nil {
		steps = []step{s}
	}
	for _, s := range steps {
		r, err := s.request()
		if err != nil {
			return err
		}
		if err := c.Send(r); err != nil {
			return err
		}
	}
	for i, s := range steps {
		a, err := c.Receive()
		if err != nil {
			return fmt.Errorf("answer %d: %w", i, err)
		}
		if err := check(s.Expect, a); err != nil {
			return fmt.Errorf("answer %d: %w", i, err)
		}
	}
	if s.Closed && !c.Closed() {
		return fmt.Errorf("server kept the connection open")
	}
	return nil
}
