package main

import (
	"os"

	"golang.org/x/sys/unix"
)

// datasync is the writer's sync_data.
func datasync(f *os.File) error { return unix.Fdatasync(int(f.Fd())) }
