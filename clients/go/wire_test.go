package segmentlog

// Checks the client against conformance/wire.txt, the golden bytes written from protocol.md that
// the Rust server tests check too. The client encodes requests and decodes answers, so it checks
// those directions; the server checks the others.

import (
	"bytes"
	"encoding/hex"
	"os"
	"reflect"
	"slices"
	"strings"
	"testing"
)

func vectors(t *testing.T) (names []string, wire map[string][]byte) {
	t.Helper()
	data, err := os.ReadFile("../../conformance/wire.txt")
	if err != nil {
		t.Fatal(err)
	}
	wire = map[string][]byte{}
	for _, line := range strings.Split(string(data), "\n") {
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		name, digits, ok := strings.Cut(line, ":")
		b, err := hex.DecodeString(strings.ReplaceAll(digits, " ", ""))
		if !ok || err != nil {
			t.Fatalf("wire.txt line %q: %v", line, err)
		}
		names = append(names, name)
		wire[name] = b
	}
	return names, wire
}

func u64(v uint64) *uint64 { return &v }

func TestWireVectors(t *testing.T) {
	names, wire := vectors(t)
	requests := map[string]Request{
		"hello": Hello(1),
		"append": Append([]NewRecord{
			{Stream: []byte("s"), Expected: Expected{Empty: true}, Payload: []byte("p")},
			{Stream: []byte("t"), Expected: Expected{Revision: 7}, Key: []byte("k"), Payload: []byte{}},
		}),
		"read":        Read(3, 9),
		"watermark":   Watermark(),
		"stream-head": StreamHead([]byte("orders")),
		"seal":        Seal(),
	}
	answers := map[string]Answer{
		"hello-ok":      {Kind: 0x81, Version: 1, FrameCap: 65536},
		"append-ok":     {Kind: 0x82, Deduplicated: true, Positions: []Position{{5, 2}}},
		"append-ok-two": {Kind: 0x82, Positions: []Position{{5, 2}, {6, 0}}},
		"read-ok-two": {Kind: 0x83, Records: []Record{
			{Offset: 1, Revision: 0, Stream: []byte("s"), Key: []byte("k"), Payload: []byte("pp")},
			{Offset: 2, Revision: 1, Stream: []byte("s"), Payload: []byte("q")},
		}},
		"read-empty":            {Kind: 0x83},
		"too-large":             {Kind: 0x82, Status: 2},
		"sealed":                {Kind: 0x82, Status: 4},
		"poisoned":              {Kind: 0x82, Status: 5},
		"exhausted":             {Kind: 0x82, Status: 6},
		"partial-retry":         {Kind: 0x82, Status: 9},
		"indeterminate":         {Kind: 0x82, Status: 10},
		"poisoned-before-write": {Kind: 0x82, Status: 11},
		"read-ok":               {Kind: 0x83, Records: []Record{{Offset: 1, Revision: 0, Stream: []byte("s"), Key: []byte("k"), Payload: []byte("pp")}}},
		"watermark-ok":          {Kind: 0x84, Watermark: 9},
		"stream-head-present":   {Kind: 0x85, Head: &Head{Revision: 3, Offset: 4}},
		"stream-head-absent":    {Kind: 0x85},
		"seal-ok":               {Kind: 0x86},
		"full":                  {Kind: 0x82, Status: 1},
		"invalid":               {Kind: 0x82, Status: 3, Reason: "ab"},
		"wrong-revision":        {Kind: 0x82, Status: 7, Index: 2, Actual: u64(6)},
		"wrong-revision-none":   {Kind: 0x82, Status: 7, Index: 0},
		"key-conflict":          {Kind: 0x82, Status: 8, Index: 3},
		"corrupt":               {Kind: 0x83, Status: 12, Segment: 1, Position: 64},
		"io":                    {Kind: 0x83, Status: 13, IoKind: "Other"},
		"unsupported-version":   {Kind: 0x81, Status: 15, Version: 1},
		"malformed":             {Kind: 0x80, Status: 14},
	}
	// The status names from protocol.md's table. The numbers above come straight off the wire,
	// so only these catch a wrong entry in the client's name table.
	statusOf := map[string]string{
		"full": "Full", "too-large": "TooLarge", "invalid": "Invalid", "sealed": "Sealed",
		"poisoned": "Poisoned", "exhausted": "Exhausted", "wrong-revision": "WrongRevision",
		"wrong-revision-none": "WrongRevision", "key-conflict": "KeyConflict",
		"partial-retry": "PartialRetry", "indeterminate": "Indeterminate",
		"poisoned-before-write": "PoisonedBeforeWrite", "corrupt": "Corrupt", "io": "Io",
		"malformed": "Malformed", "unsupported-version": "UnsupportedVersion",
	}
	for name, want := range answers {
		got := want.Status.String()
		if want.Status == 0 {
			got = ""
		}
		if got != statusOf[name] {
			t.Errorf("%s: status %d is named %q, want %q", name, want.Status, got, statusOf[name])
		}
	}
	var checked []string
	for _, name := range names {
		b := wire[name]
		if r, ok := requests[name]; ok {
			if !bytes.Equal(r, b) {
				t.Errorf("%s: encoded %x, want %x", name, []byte(r), b)
			}
		} else if want, ok := answers[name]; ok {
			got, err := decode(b[4], b[headerLen:])
			if err != nil || !reflect.DeepEqual(got, want) {
				t.Errorf("%s: decoded %+v (%v), want %+v", name, got, err, want)
			}
		} else {
			t.Errorf("%s: no check for this vector", name)
		}
		checked = append(checked, name)
	}
	for name := range requests {
		if !slices.Contains(checked, name) {
			t.Errorf("request check %s has no vector", name)
		}
	}
	for name := range answers {
		if !slices.Contains(checked, name) {
			t.Errorf("answer check %s has no vector", name)
		}
	}
}
