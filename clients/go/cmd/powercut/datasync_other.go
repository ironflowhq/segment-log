//go:build !linux

package main

import "os"

// datasync falls back to fsync where the probe is only built, not qualified.
func datasync(f *os.File) error { return f.Sync() }
