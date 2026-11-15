package segmentlog

import (
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"

	"golang.org/x/sys/unix"
)

// dialVsock connects to "cid:port". The net package has no vsock, so the connection is an
// os.File over the socket. A non-blocking descriptor puts it on Go's poller, so Close interrupts
// a blocked read.
func dialVsock(address string) (io.ReadWriteCloser, error) {
	cid, port, ok := strings.Cut(address, ":")
	c, err1 := strconv.ParseUint(cid, 10, 32)
	p, err2 := strconv.ParseUint(port, 10, 32)
	if !ok || err1 != nil || err2 != nil {
		return nil, fmt.Errorf("vsock address %q: want cid:port", address)
	}
	fd, err := unix.Socket(unix.AF_VSOCK, unix.SOCK_STREAM|unix.SOCK_CLOEXEC, 0)
	if err != nil {
		return nil, fmt.Errorf("vsock socket: %w", err)
	}
	if err := unix.Connect(fd, &unix.SockaddrVM{CID: uint32(c), Port: uint32(p)}); err != nil {
		unix.Close(fd)
		return nil, fmt.Errorf("vsock connect %s: %w", address, err)
	}
	if err := unix.SetNonblock(fd, true); err != nil {
		unix.Close(fd)
		return nil, err
	}
	return os.NewFile(uintptr(fd), "vsock:"+address), nil
}
