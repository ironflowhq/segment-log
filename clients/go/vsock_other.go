//go:build !linux

package segmentlog

import (
	"errors"
	"io"
)

func dialVsock(string) (io.ReadWriteCloser, error) { return nil, errors.New("vsock needs Linux") }
