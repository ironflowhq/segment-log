package segmentlog

// The host side of Firecracker's hybrid vsock (ADR 0059). A fake VMM socket stands in for
// Firecracker, so these run on any host.

import (
	"bufio"
	"io"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// fakeVMM listens on a Unix socket like Firecracker's vsock uds_path. It reads one line, answers
// with reply, and then echoes the connection if the reply accepted it.
func fakeVMM(t *testing.T, reply string) (path string, got chan string) {
	t.Helper()
	dir, err := os.MkdirTemp("/tmp", "fc:") // a colon in the path, as a socket path may have
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	path = filepath.Join(dir, "v.sock")
	l, err := net.Listen("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { l.Close() })
	got = make(chan string, 1)
	go func() {
		c, err := l.Accept()
		if err != nil {
			return
		}
		defer c.Close()
		line, _ := bufio.NewReader(c).ReadString('\n')
		got <- line
		if _, err := io.WriteString(c, reply); err != nil || !strings.HasPrefix(reply, "OK ") {
			return
		}
		io.Copy(c, c)
	}()
	return path, got
}

func TestFirecrackerVsockConnects(t *testing.T) {
	// Given a VMM socket that accepts the guest port
	path, got := fakeVMM(t, "OK 1073741824\n")

	// When the client dials it with the guest port
	conn, err := dialFirecracker(path + ":5000")
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()

	// Then it sent the handshake, and the bytes after it reach the guest
	if line := <-got; line != "CONNECT 5000\n" {
		t.Fatalf("handshake %q, want %q", line, "CONNECT 5000\n")
	}
	if _, err := conn.Write([]byte("ping")); err != nil {
		t.Fatal(err)
	}
	buf := make([]byte, 4)
	if _, err := io.ReadFull(conn, buf); err != nil || string(buf) != "ping" {
		t.Fatalf("echo %q, %v", buf, err)
	}
}

func TestFirecrackerVsockRefused(t *testing.T) {
	// Given a VMM socket with no guest listener on the port: Firecracker closes without OK
	path, _ := fakeVMM(t, "")

	// When the client dials it
	_, err := dialFirecracker(path + ":5000")

	// Then the dial fails rather than handing back a dead connection
	if err == nil {
		t.Fatal("dial succeeded without OK")
	}
}

func TestFirecrackerVsockBadAddress(t *testing.T) {
	for _, address := range []string{"no-port", "/tmp/v.sock:", "/tmp/v.sock:x", "/tmp/v.sock:-1"} {
		if _, err := dialFirecracker(address); err == nil {
			t.Errorf("%q: dial succeeded", address)
		}
	}
}
