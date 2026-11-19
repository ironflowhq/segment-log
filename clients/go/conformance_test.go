package segmentlog

// Runs the shared conformance cases in conformance/*.json against a server binary. The cases are
// data, so every transport runs the same ones (testing.md, M6 transport order and conformance).
//
// SEGMENT_LOG_SERVER names the binary: cargo build -p segment-log-server, then
// SEGMENT_LOG_SERVER=$PWD/target/debug/segment-log-server go test ./...
//
// SEGMENT_LOG_TRANSPORT=vsock runs every test over the Linux vsock loopback (CID 1) instead of
// a Unix socket. It needs the vsock_loopback module; without it the tests fail, never skip.
//
// SEGMENT_LOG_TRANSPORT=firecracker runs them over a Firecracker guest's vsock, from the host
// side; scripts/m7-rehearse.sh sets it up.

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"sync/atomic"
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
	Hello      *struct {
		Min uint16 `json:"min"`
		Max uint16 `json:"max"`
	} `json:"hello"`
	Raw      string  `json:"raw"`
	Pipeline []step  `json:"pipeline"`
	Expect   *expect `json:"expect"`
	Closed   bool    `json:"closed"`
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
	Version      *uint16         `json:"version"`
	MinVersion   *uint16         `json:"min_version"`
	MaxVersion   *uint16         `json:"max_version"`
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
	case s.Hello != nil:
		return Hello(s.Hello.Min, s.Hello.Max), nil
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
	for _, f := range []struct {
		name string
		want *uint16
		got  uint16
	}{{"version", e.Version, a.Version}, {"min_version", e.MinVersion, a.MinVersion}, {"max_version", e.MaxVersion, a.MaxVersion}} {
		if f.want != nil && f.got != *f.want {
			return fmt.Errorf("%s %d, want %d", f.name, f.got, *f.want)
		}
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

// logDir is a log directory and a listening address that outlive one server process.
type logDir struct {
	dir string
	// network and address are what the client dials; listen is the server's flags for them.
	network, address string
	listen           []string
}

// vsockPort gives each log directory its own port. The pid keeps two test processes on one
// host apart.
var vsockPort atomic.Uint32

func init() { vsockPort.Store(20000 + uint32(os.Getpid()%1000)*40) }

func newLogDir(t *testing.T) logDir {
	t.Helper()
	// Under /tmp, not t.TempDir: macOS limits a socket path to 104 bytes.
	dir, err := os.MkdirTemp("/tmp", "sl-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	switch transport := os.Getenv("SEGMENT_LOG_TRANSPORT"); transport {
	case "", "unix":
		socket := filepath.Join(dir, "s")
		return logDir{dir, "unix", socket, []string{"--unix", socket}}
	case "vsock":
		port := fmt.Sprint(vsockPort.Add(1))
		return logDir{dir, "vsock", "1:" + port, []string{"--vsock-port", port}}
	case "firecracker":
		// SEGMENT_LOG_SERVER is scripts/m7/fc-server, which boots a guest per start (ADR 0059).
		return logDir{dir, "firecracker", filepath.Join(dir, "v.sock") + ":5000", []string{"--vsock-port", "5000"}}
	default:
		t.Fatalf("SEGMENT_LOG_TRANSPORT=%s: want unix, vsock or firecracker", transport)
		return logDir{}
	}
}

// start runs the server on d and returns once it listens. A non-empty shell prefix runs before
// exec, for example to set a resource limit. The test cleanup kills the process.
func (d logDir) start(t *testing.T, shell string, args ...string) *exec.Cmd {
	t.Helper()
	bin := os.Getenv("SEGMENT_LOG_SERVER")
	if bin == "" {
		t.Fatal("set SEGMENT_LOG_SERVER to the server binary (cargo build -p segment-log-server)")
	}
	argv := append(append([]string{bin, "--dir", filepath.Join(d.dir, "log")}, d.listen...), args...)
	if shell != "" {
		argv = append([]string{"sh", "-c", shell + `; exec "$0" "$@"`}, argv...)
	}
	var stderr bytes.Buffer
	cmd := exec.Command(argv[0], argv[1:]...)
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
		// A socket left by an earlier process refuses connections until the new one binds.
		if c, err := DialRaw(d.network, d.address); err == nil {
			c.Close()
			return cmd
		}
	}
	t.Fatalf("server did not listen on %s; stderr:\n%s", d.address, stderr.String())
	return nil
}

func TestConformance(t *testing.T) {
	files, err := filepath.Glob("../../conformance/*.json")
	if err != nil || len(files) == 0 {
		t.Fatalf("no conformance cases found: %v", err)
	}
	versions := serverVersions(t)
	for _, file := range files {
		data, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		var c caseFile
		if err := json.Unmarshal(data, &c); err != nil {
			t.Fatalf("%s: %v", file, err)
		}
		// A case that sends its own Hello runs once; every other case runs at each version.
		if c.NoHello {
			t.Run(filepath.Base(file), func(t *testing.T) {
				d := newLogDir(t)
				d.start(t, "", c.ServerArgs...)
				runCase(t, c, func() (*Client, error) { return DialRaw(d.network, d.address) })
			})
			continue
		}
		for _, v := range versions {
			t.Run(fmt.Sprintf("%s/v%d", filepath.Base(file), v), func(t *testing.T) {
				d := newLogDir(t)
				d.start(t, "", c.ServerArgs...)
				runCase(t, c, func() (*Client, error) { return DialVersions(d.network, d.address, v, v) })
			})
		}
	}
}

func runCase(t *testing.T, c caseFile, dial func() (*Client, error)) {
	t.Helper()
	client, err := dial()
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	for i, s := range c.Steps {
		if err := run(client, s); err != nil {
			t.Fatalf("%s (%s) step %d: %v", c.Name, c.Case, i, err)
		}
	}
}

// serverVersions asks a server for the versions it claims: a Hello that offers no version gets
// UnsupportedVersion with the server's range (protocol.md, Hello). Each must be one the client
// speaks, so a server that adds a version fails here until the client and its cases follow.
func serverVersions(t *testing.T) []uint16 {
	t.Helper()
	d := newLogDir(t)
	d.start(t, "")
	c, err := DialRaw(d.network, d.address)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	a, err := c.Do(Hello(1, 0))
	if err != nil || a.Status.String() != "UnsupportedVersion" {
		t.Fatalf("probe Hello: %+v, %v", a, err)
	}
	var versions []uint16
	for v := a.MinVersion; v <= a.MaxVersion; v++ {
		if v < MinVersion || v > MaxVersion {
			t.Fatalf("server claims version %d; the client speaks %d to %d", v, MinVersion, MaxVersion)
		}
		versions = append(versions, v)
	}
	if len(versions) == 0 {
		t.Fatalf("server claims no version: %+v", a)
	}
	return versions
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

// TestConformancePool runs the cases through a pool of one connection (issue 134), so a
// pipeline's requests keep their order. It skips the cases that send their own Hello or raw bytes,
// or that wait for the server to close the connection: those test the server's session, not the
// pool.
func TestConformancePool(t *testing.T) {
	files, err := filepath.Glob("../../conformance/*.json")
	if err != nil || len(files) == 0 {
		t.Fatalf("no conformance cases found: %v", err)
	}
	ran := 0
	for _, file := range files {
		data, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		var c caseFile
		if err := json.Unmarshal(data, &c); err != nil {
			t.Fatalf("%s: %v", file, err)
		}
		if c.NoHello || slices.ContainsFunc(c.Steps, sessionStep) {
			continue
		}
		ran++
		t.Run(filepath.Base(file), func(t *testing.T) {
			d := newLogDir(t)
			d.start(t, "", c.ServerArgs...)
			p, err := DialPool(d.network, d.address, 1)
			if err != nil {
				t.Fatal(err)
			}
			defer p.Close()
			for i, s := range c.Steps {
				steps := s.Pipeline
				if steps == nil {
					steps = []step{s}
				}
				var replies []<-chan poolResult
				for _, s := range steps {
					r, err := s.request()
					if err != nil {
						t.Fatal(err)
					}
					replies = append(replies, p.send(r))
				}
				for j, reply := range replies {
					res := <-reply
					if res.err == nil {
						res.err = check(steps[j].Expect, res.answer)
					}
					if res.err != nil {
						t.Fatalf("%s (%s) step %d answer %d: %v", c.Name, c.Case, i, j, res.err)
					}
				}
			}
		})
	}
	if ran == 0 {
		t.Fatal("no conformance case runs through the pool")
	}
}

func sessionStep(s step) bool {
	return s.Raw != "" || s.Closed || s.Hello != nil || slices.ContainsFunc(s.Pipeline, sessionStep)
}
