package segmentlog

import (
	"fmt"
	"net"
	"strconv"
	"strings"
)

// dialFirecracker connects to a guest port through Firecracker's hybrid vsock: the VMM's Unix
// socket takes "CONNECT <port>\n" and answers "OK <host port>\n" (ADR 0059). The address is
// "<socket path>:<port>"; the last colon splits it, since a path may hold one.
func dialFirecracker(address string) (net.Conn, error) {
	i := strings.LastIndexByte(address, ':')
	if i <= 0 {
		return nil, fmt.Errorf("firecracker address %q: want <socket path>:<port>", address)
	}
	path := address[:i]
	port, err := strconv.ParseUint(address[i+1:], 10, 32)
	if err != nil {
		return nil, fmt.Errorf("firecracker address %q: want <socket path>:<port>", address)
	}
	conn, err := net.Dial("unix", path)
	if err != nil {
		return nil, err
	}
	if _, err := fmt.Fprintf(conn, "CONNECT %d\n", port); err != nil {
		conn.Close()
		return nil, err
	}
	// One byte at a time: a buffered read could swallow the first bytes from the guest.
	var line []byte
	b := make([]byte, 1)
	for len(line) < 32 {
		if _, err := conn.Read(b); err != nil {
			conn.Close()
			return nil, fmt.Errorf("firecracker connect %s: no guest listener (%w)", address, err)
		}
		if b[0] == '\n' {
			if !strings.HasPrefix(string(line), "OK ") {
				break
			}
			return conn, nil
		}
		line = append(line, b[0])
	}
	conn.Close()
	return nil, fmt.Errorf("firecracker connect %s: answer %q", address, line)
}
