package segmentlog

import (
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"
	"time"

	"golang.org/x/sys/unix"
)

// dialVsock connects to "cid:port". The net package has no vsock, so the connection is an
// os.File over the socket. A non-blocking descriptor puts it on Go's poller, so Close interrupts
// a blocked read.
//
// The connect is non-blocking, as in net: a blocking connect returns EINTR when a signal (the
// runtime's own SIGURG) lands on its thread, and retrying it finds the first attempt still
// running (issue 187). The kernel's vsock connect timeout, 2 s by default, bounds the wait.
func dialVsock(address string) (io.ReadWriteCloser, error) {
	cid, port, ok := strings.Cut(address, ":")
	c, err1 := strconv.ParseUint(cid, 10, 32)
	p, err2 := strconv.ParseUint(port, 10, 32)
	if !ok || err1 != nil || err2 != nil {
		return nil, fmt.Errorf("vsock address %q: want cid:port", address)
	}
	fd, err := unix.Socket(unix.AF_VSOCK, unix.SOCK_STREAM|unix.SOCK_CLOEXEC|unix.SOCK_NONBLOCK, 0)
	if err != nil {
		return nil, fmt.Errorf("vsock socket: %w", err)
	}
	sa := &unix.SockaddrVM{CID: uint32(c), Port: uint32(p)}
	for { // a non-blocking vsock connect does not sleep, so EINTR is not expected; net retries it too
		if err = unix.Connect(fd, sa); err != unix.EINTR {
			break
		}
	}
	switch err {
	case nil, unix.EINPROGRESS, unix.EALREADY, unix.EISCONN:
	default:
		unix.Close(fd)
		return nil, fmt.Errorf("vsock connect %s: %w", address, err)
	}
	f := os.NewFile(uintptr(fd), "vsock:"+address)
	if err == nil || err == unix.EISCONN {
		return f, nil
	}
	if err := awaitConnect(f); err != nil {
		f.Close()
		return nil, fmt.Errorf("vsock connect %s: %w", address, err)
	}
	return f, nil
}

// connectBackstop ends a wait that the kernel's connect timeout should already have ended, should
// the socket stop connecting without an error to report.
const connectBackstop = 10 * time.Second

// awaitConnect waits on the poller until a connect in progress ends. A connected socket has a
// peer; asking for it, not connecting again, keeps a failed attempt from starting a new one.
func awaitConnect(f *os.File) error {
	raw, err := f.SyscallConn()
	if err != nil {
		return err
	}
	if err := f.SetWriteDeadline(time.Now().Add(connectBackstop)); err != nil {
		return err
	}
	var result error
	err = raw.Write(func(fd uintptr) bool {
		n, err := unix.GetsockoptInt(int(fd), unix.SOL_SOCKET, unix.SO_ERROR)
		if err != nil {
			result = err
			return true
		}
		if n != 0 {
			result = unix.Errno(n)
			return true
		}
		// The poller can wake before the connect ends, and runs this once before any wait.
		if _, err := unix.Getpeername(int(fd)); err != unix.ENOTCONN {
			result = err
			return true
		}
		return false
	})
	if err != nil {
		return err
	}
	if result != nil {
		return result
	}
	return f.SetWriteDeadline(time.Time{}) // the connection's writes have no deadline
}
