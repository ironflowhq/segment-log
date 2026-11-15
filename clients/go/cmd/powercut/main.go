// Command powercut is the power-cut harness (volume-requirements.md, ADR 0040, ADR 0059).
//
//	powercut observe -listen :7070 -dir <store>      on a machine that the event does not touch
//	powercut load  -network N -address A -observer H:P -run R -cycle C [-report-early]
//	powercut check -network N -address A -observer H:P -run R
//	powercut ready -network N -address A                  waits until the server answers Hello
//
// load appends fully keyed records with deterministic payloads and, after each acknowledgment,
// reports its key, log offset and stream revision to the observer, which stores them durably.
// A record that is durable but never reported is allowed: the unknown suffix of the commit
// protocol. check reads the whole log back after recovery and fails if a reported record is
// missing or moved (I3), or if the log offsets have a gap (I4).
//
// -report-early is the known-bad control for process crash, guest crash and guest destroy: it
// reports each record when it is sent, before its acknowledgment, as a writer that acknowledges
// before it writes would. The harness must then see missing records.
package main

import (
	"bufio"
	"bytes"
	"flag"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	segmentlog "github.com/ironflowhq/segment-log/clients/go"
)

func main() {
	if len(os.Args) < 2 {
		fail("usage: powercut observe|load|check [flags]")
	}
	switch os.Args[1] {
	case "observe":
		observe(os.Args[2:])
	case "load":
		load(os.Args[2:])
	case "check":
		check(os.Args[2:])
	case "ready":
		ready(os.Args[2:])
	default:
		fail("unknown command " + os.Args[1])
	}
}

func fail(msg string) {
	fmt.Fprintln(os.Stderr, "powercut:", msg)
	os.Exit(2)
}

// payload is a pure function of the key, so check needs nothing but the key to verify it.
func payload(key string, size int) []byte {
	return bytes.Repeat([]byte(key+"|"), size/(len(key)+1)+1)[:size]
}

// --- observer ---

// observe stores report lines per run, one file each, and syncs before it confirms. Protocol, one
// line each: "REPORT <run>" then report lines and "SYNC" (answered "ok"); or "DUMP <run>", answered
// with the stored lines.
func observe(args []string) {
	fs := flag.NewFlagSet("observe", flag.ExitOnError)
	listen := fs.String("listen", ":7070", "address to listen on")
	dir := fs.String("dir", "powercut-store", "directory for the stored reports")
	fs.Parse(args)
	if err := os.MkdirAll(*dir, 0o755); err != nil {
		fail(err.Error())
	}
	l, err := net.Listen("tcp", *listen)
	if err != nil {
		fail(err.Error())
	}
	fmt.Fprintln(os.Stderr, "powercut: observing on", l.Addr())
	var mu sync.Mutex // one writer per file at a time; DUMP sees whole lines
	for {
		c, err := l.Accept()
		if err != nil {
			continue
		}
		go serveObserver(c, *dir, &mu)
	}
}

func serveObserver(c net.Conn, dir string, mu *sync.Mutex) {
	defer c.Close()
	r := bufio.NewReader(c)
	first, err := r.ReadString('\n')
	if err != nil {
		return
	}
	verb, run, _ := strings.Cut(strings.TrimSpace(first), " ")
	if run == "" || strings.ContainsAny(run, "/\\.") {
		return
	}
	path := filepath.Join(dir, run+".log")
	switch verb {
	case "DUMP":
		mu.Lock()
		data, _ := os.ReadFile(path)
		mu.Unlock()
		c.Write(data)
	case "REPORT":
		f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o644)
		if err != nil {
			return
		}
		defer f.Close()
		var batch []byte
		for {
			line, err := r.ReadString('\n')
			sync := line == "SYNC\n"
			if !sync && strings.HasSuffix(line, "\n") {
				batch = append(batch, line...)
			}
			// Group commit: one sync for whatever has arrived.
			if sync || err != nil || r.Buffered() == 0 {
				mu.Lock()
				f.Write(batch)
				f.Sync()
				mu.Unlock()
				batch = batch[:0]
				if sync {
					io.WriteString(c, "ok\n")
				}
				if err != nil {
					return
				}
			}
		}
	}
}

// --- load ---

func load(args []string) {
	fs := flag.NewFlagSet("load", flag.ExitOnError)
	network := fs.String("network", "firecracker", "client network")
	address := fs.String("address", "", "server address")
	observer := fs.String("observer", "", "observer host:port")
	run := fs.String("run", "", "run id")
	cycle := fs.Int("cycle", 0, "cycle number; each cycle uses new streams")
	workers := fs.Int("workers", 16, "concurrent writers, one connection and stream each")
	size := fs.Int("payload", 256, "payload bytes")
	limit := fs.Duration("max", 60*time.Second, "stop after this long if no event ends the load")
	interval := fs.Duration("interval", 2*time.Millisecond, "pause between one worker's appends, for a steady load")
	early := fs.Bool("report-early", false, "known-bad control: report before the acknowledgment")
	fs.Parse(args)

	obs, err := net.Dial("tcp", *observer)
	if err != nil {
		fail(err.Error())
	}
	fmt.Fprintf(obs, "REPORT %s\n", *run)
	reports := make(chan string, 1024)
	sent := make(chan int)
	go func() {
		w := bufio.NewWriter(obs)
		n := 0
		for line := range reports {
			w.WriteString(line)
			n++
			if len(reports) == 0 {
				w.Flush()
			}
		}
		w.Flush()
		sent <- n
	}()

	deadline := time.Now().Add(*limit)
	var wg sync.WaitGroup
	acked := make([]int, *workers)
	var interrupted atomic.Bool // set when the event ends a worker; a cycle without it is void
	for w := range *workers {
		wg.Add(1)
		go func() {
			defer wg.Done()
			c, err := segmentlog.Dial(*network, *address)
			if err != nil {
				return
			}
			defer c.Close()
			stream := fmt.Sprintf("c%d-w%d", *cycle, w)
			for i := 0; time.Now().Before(deadline); i++ {
				key := fmt.Sprintf("%s/%d/%d/%d", *run, *cycle, w, i)
				expected := segmentlog.Expected{Empty: i == 0, Revision: uint64(max(i-1, 0))}
				rec := segmentlog.NewRecord{Stream: []byte(stream), Expected: expected, Key: []byte(key), Payload: payload(key, *size)}
				if *early {
					reports <- fmt.Sprintf("%s -1 %d\n", key, i)
				}
				a, err := c.Do(segmentlog.Append([]segmentlog.NewRecord{rec}))
				if err != nil {
					interrupted.Store(true)
					return // the event: the outcome of this one is unknown
				}
				if a.Status != 0 {
					fmt.Fprintf(os.Stderr, "powercut: %s: %v\n", key, a.Status)
					return
				}
				if !*early {
					reports <- fmt.Sprintf("%s %d %d\n", key, a.Positions[0].Offset, a.Positions[0].Revision)
				}
				acked[w]++
				time.Sleep(*interval)
			}
		}()
	}
	wg.Wait()
	close(reports)
	n := <-sent
	// Everything reported is stored before the cycle's check reads it back.
	io.WriteString(obs, "SYNC\n")
	if reply, err := bufio.NewReader(obs).ReadString('\n'); err != nil || reply != "ok\n" {
		fail(fmt.Sprintf("observer did not confirm: %q %v", reply, err))
	}
	total := 0
	for _, a := range acked {
		total += a
	}
	fmt.Printf("load: cycle %d: %d acknowledged, %d reported\n", *cycle, total, n)
	if !interrupted.Load() {
		fail("no event ended the load, so this cycle tests nothing")
	}
}

// --- check ---

func check(args []string) {
	fs := flag.NewFlagSet("check", flag.ExitOnError)
	network := fs.String("network", "firecracker", "client network")
	address := fs.String("address", "", "server address")
	observer := fs.String("observer", "", "observer host:port")
	run := fs.String("run", "", "run id")
	size := fs.Int("payload", 256, "payload bytes")
	only := fs.Int("cycle", 0, "count only this cycle's reports (the control); 0 counts every cycle")
	fs.Parse(args)

	c, err := segmentlog.Dial(*network, *address)
	if err != nil {
		fail(err.Error())
	}
	defer c.Close()
	byKey := map[string]segmentlog.Record{}
	var total uint64
	for {
		a, err := c.Do(segmentlog.Read(total, 1000))
		if err != nil || a.Status != 0 {
			fail(fmt.Sprintf("read at %d: %v %v", total, a.Status, err))
		}
		if len(a.Records) == 0 {
			break
		}
		for _, r := range a.Records {
			if r.Offset != total {
				fail(fmt.Sprintf("I4: offset %d where %d was next", r.Offset, total))
			}
			total++
			key := string(r.Key)
			if _, dup := byKey[key]; dup {
				fail("key twice in the log: " + key)
			}
			if !bytes.Equal(r.Payload, payload(key, *size)) {
				fail("payload does not match its key: " + key)
			}
			byKey[key] = r
		}
	}

	obs, err := net.Dial("tcp", *observer)
	if err != nil {
		fail(err.Error())
	}
	fmt.Fprintf(obs, "DUMP %s\n", *run)
	dump, err := io.ReadAll(obs)
	if err != nil {
		fail(err.Error())
	}
	reported, missing := 0, 0
	for _, line := range strings.Split(strings.TrimSpace(string(dump)), "\n") {
		if line == "" {
			continue
		}
		f := strings.Fields(line)
		if len(f) != 3 {
			fail("bad report line: " + line)
		}
		offset, _ := strconv.ParseInt(f[1], 10, 64)
		revision, _ := strconv.ParseUint(f[2], 10, 64)
		if *only != 0 && strings.Split(f[0], "/")[1] != strconv.Itoa(*only) {
			continue
		}
		reported++
		r, ok := byKey[f[0]]
		if !ok || r.Revision != revision || (offset >= 0 && r.Offset != uint64(offset)) {
			missing++
			if missing <= 5 {
				fmt.Printf("check: MISSING %s (want offset %d revision %d; found %v)\n", f[0], offset, revision, ok)
			}
		}
	}
	fmt.Printf("check: %d records, %d reported, %d missing\n", total, reported, missing)
	if missing > 0 {
		os.Exit(1)
	}
}

func ready(args []string) {
	fs := flag.NewFlagSet("ready", flag.ExitOnError)
	network := fs.String("network", "firecracker", "client network")
	address := fs.String("address", "", "server address")
	limit := fs.Duration("max", 30*time.Second, "give up after this long")
	fs.Parse(args)
	for deadline := time.Now().Add(*limit); time.Now().Before(deadline); time.Sleep(50 * time.Millisecond) {
		if c, err := segmentlog.Dial(*network, *address); err == nil {
			c.Close()
			return
		}
	}
	fail("the server did not answer at " + *address)
}
