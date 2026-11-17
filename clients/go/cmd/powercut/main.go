// Command powercut is the power-cut harness (volume-requirements.md, ADR 0040, ADR 0059).
//
//	powercut observe -listen :7070 -dir <store>      on a machine that the event does not touch
//	powercut load  -network N -address A -observer H:P -run R -cycle C [-report-early]
//	powercut check -network N -address A -observer H:P -run R
//	powercut ready -network N -address A                  waits until the server answers Hello
//	powercut probe -dir D -bytes B -duration T -rate R      the flush probe (VOL-FLUSH-LATENCY)
//	powercut metaerr -dir D                                 the steps of VOL-METADATA-ERROR
//	powercut seal -network N -address A                     seals the log
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
	"slices"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	segmentlog "github.com/ironflowx/segment-log/clients/go"
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
	case "probe":
		probe(os.Args[2:])
	case "metaerr":
		metaerr(os.Args[2:])
	case "seal":
		seal(os.Args[2:])
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

// probe is the flush probe of volume-requirements.md: append one group-sized write to the end of
// a new file, then fdatasync, so that each sync also commits the new size, as the writer's
// commit does. It prints one PROBE line with the cycle statistics after the warm-up.
func probe(args []string) {
	fs := flag.NewFlagSet("probe", flag.ExitOnError)
	dir := fs.String("dir", ".", "directory on the volume under test")
	size := fs.Int("bytes", 16384, "bytes per write: the expected encoded group")
	duration := fs.Duration("duration", 60*time.Second, "measured time, after the warm-up")
	warmup := fs.Duration("warmup", 5*time.Second, "unmeasured time first")
	rate := fs.Float64("rate", 0, "cycles per second, 0 for no limit")
	fs.Parse(args)
	f, err := os.Create(filepath.Join(*dir, "flush-probe"))
	if err != nil {
		fail(err.Error())
	}
	defer f.Close()
	block := bytes.Repeat([]byte{0x5a}, *size)
	var cycles []time.Duration
	start := time.Now()
	next := start
	for {
		now := time.Now()
		if now.Sub(start) >= *warmup+*duration {
			break
		}
		if *rate > 0 {
			time.Sleep(time.Until(next))
			next = next.Add(time.Duration(float64(time.Second) / *rate))
		}
		began := time.Now()
		if _, err := f.Write(block); err != nil {
			fail(err.Error())
		}
		if err := datasync(f); err != nil {
			fail(err.Error())
		}
		if began.Sub(start) >= *warmup {
			cycles = append(cycles, time.Since(began))
		}
	}
	if len(cycles) == 0 {
		fail("no cycle after the warm-up")
	}
	slices.Sort(cycles)
	var sum time.Duration
	for _, c := range cycles {
		sum += c
	}
	at := func(q float64) int64 { return cycles[min(len(cycles)-1, int(q*float64(len(cycles))))].Microseconds() }
	fmt.Printf("PROBE cycles=%d syncs_per_s=%.1f mean_us=%d p50_us=%d p99_us=%d p999_us=%d max_us=%d bytes=%d\n",
		len(cycles), float64(len(cycles))/duration.Seconds(), (sum / time.Duration(len(cycles))).Microseconds(),
		at(0.5), at(0.99), at(0.999), cycles[len(cycles)-1].Microseconds(), *size)
}

// metaerr runs the four steps of VOL-METADATA-ERROR (volume-requirements.md, V3) in dir. It prints
// READY-FAIL and READY-RECOVER and waits for a line on stdin at each, while the host switches the
// device under the filesystem to failing writes and back. Each later line reports one call.
func metaerr(args []string) {
	fs := flag.NewFlagSet("metaerr", flag.ExitOnError)
	dir := fs.String("dir", ".", "directory on the filesystem under test")
	fs.Parse(args)
	in := bufio.NewReader(os.Stdin)
	report := func(step string, err error) {
		if err != nil {
			fmt.Printf("STEP %s err %v\n", step, err)
		} else {
			fmt.Printf("STEP %s ok\n", step)
		}
	}
	writeSynced := func(name string) error {
		f, err := os.Create(filepath.Join(*dir, name))
		if err != nil {
			return err
		}
		defer f.Close()
		if _, err := f.Write([]byte(name)); err != nil {
			return err
		}
		return f.Sync()
	}
	wait := func(cue string) {
		fmt.Println(cue)
		if _, err := in.ReadString('\n'); err != nil {
			fail("no answer to " + cue)
		}
	}
	if err := os.MkdirAll(*dir, 0o755); err != nil {
		fail(err.Error())
	}
	// Step 1: a durable file.
	report("1-file", writeSynced("first"))
	// Step 2: with writes failing, a new entry and the directory fsync.
	held, err := os.Open(*dir)
	if err != nil {
		fail(err.Error())
	}
	wait("READY-FAIL")
	f, err := os.Create(filepath.Join(*dir, "second"))
	if err == nil {
		f.Close()
	}
	report("2-create", err)
	report("2-dirsync", held.Sync())
	// Step 4, after the device works again: both handles, then a file.
	wait("READY-RECOVER")
	report("4-dirsync-held", held.Sync())
	fresh, err := os.Open(*dir)
	if err == nil {
		err = fresh.Sync()
		fresh.Close()
	}
	report("4-dirsync-new", err)
	report("4-file", writeSynced("third"))
	fmt.Println("DONE")
}

func seal(args []string) {
	fs := flag.NewFlagSet("seal", flag.ExitOnError)
	network := fs.String("network", "firecracker", "client network")
	address := fs.String("address", "", "server address")
	fs.Parse(args)
	c, err := segmentlog.Dial(*network, *address)
	if err != nil {
		fail(err.Error())
	}
	defer c.Close()
	if a, err := c.Do(segmentlog.Seal()); err != nil || a.Status != 0 {
		fail(fmt.Sprintf("seal: %v %v", a.Status, err))
	}
}
