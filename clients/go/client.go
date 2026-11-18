// Package segmentlog is the Go client for segment-log-server. It runs the conformance cases and
// the socket benchmark (ADR 0058). From 0.1 it follows 0.x semver and is tagged
// clients/go/vX.Y.Z in step with the crates (ADR 0064 rule 15).
//
// docs/architecture/protocol.md is the only source for the bytes it sends and reads.
package segmentlog

import (
	"bufio"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"syscall"
)

// The protocol versions this client speaks (ADR 0064). Dial offers the whole range.
const (
	MinVersion = 2
	MaxVersion = 2
)

const (
	headerLen = 8
	answerBit = 0x80

	KindHello      = 1
	KindAppend     = 2
	KindRead       = 3
	KindWatermark  = 4
	KindStreamHead = 5
	KindSeal       = 6
)

// Status is an answer's status code. Zero is success.
type Status uint16

var statusNames = []string{
	"Ok", "Full", "TooLarge", "Invalid", "Sealed", "Poisoned", "Exhausted", "WrongRevision",
	"KeyConflict", "PartialRetry", "Indeterminate", "PoisonedBeforeWrite", "Corrupt", "Io",
	"Malformed", "UnsupportedVersion",
}

func (s Status) String() string {
	if int(s) < len(statusNames) {
		return statusNames[s]
	}
	return fmt.Sprintf("Status(%d)", uint16(s))
}

// Expected is a record's expected revision: Empty, or At(Revision).
type Expected struct {
	Empty    bool
	Revision uint64
}

// NewRecord is one record of an append. A nil Key means no key.
type NewRecord struct {
	Stream   []byte
	Expected Expected
	Key      []byte
	Payload  []byte
}

type Position struct{ Offset, Revision uint64 }

type Record struct {
	Offset, Revision uint64
	Stream, Key      []byte
	Payload          []byte
}

type Head struct{ Revision, Offset uint64 }

// Answer is one decoded answer. Only the fields for its kind and status are set.
type Answer struct {
	Kind   byte
	Status Status

	// Version is the chosen version of a Hello answer.
	Version      uint16
	FrameCap     uint32
	Deduplicated bool
	Positions    []Position
	Records      []Record
	Watermark    uint64
	Head         *Head

	Reason   string
	Index    uint32
	Actual   *uint64
	Segment  uint64
	Position uint64
	IoKind   string
	// MinVersion and MaxVersion are the server's range in an UnsupportedVersion answer.
	MinVersion, MaxVersion uint16
}

// Request is one encoded request message, header included.
type Request []byte

func message(kind byte, body []byte) Request {
	m := make([]byte, headerLen, headerLen+len(body))
	binary.LittleEndian.PutUint32(m, uint32(len(body)))
	m[4] = kind
	return append(m, body...)
}

func Hello(min, max uint16) Request {
	body := []byte("SGLP")
	body = binary.LittleEndian.AppendUint16(body, min)
	return message(KindHello, binary.LittleEndian.AppendUint16(body, max))
}

func Append(records []NewRecord) Request {
	body := binary.LittleEndian.AppendUint32(nil, uint32(len(records)))
	for _, r := range records {
		body = binary.LittleEndian.AppendUint16(body, uint16(len(r.Stream)))
		body = binary.LittleEndian.AppendUint16(body, uint16(len(r.Key)))
		body = binary.LittleEndian.AppendUint32(body, uint32(len(r.Payload)))
		if r.Expected.Empty {
			body = append(body, 0)
			body = binary.LittleEndian.AppendUint64(body, 0)
		} else {
			body = append(body, 1)
			body = binary.LittleEndian.AppendUint64(body, r.Expected.Revision)
		}
		body = append(append(append(body, r.Stream...), r.Key...), r.Payload...)
	}
	return message(KindAppend, body)
}

func Read(from uint64, max uint32) Request {
	body := binary.LittleEndian.AppendUint64(nil, from)
	return message(KindRead, binary.LittleEndian.AppendUint32(body, max))
}

func Watermark() Request               { return message(KindWatermark, nil) }
func StreamHead(stream []byte) Request { return message(KindStreamHead, stream) }
func Seal() Request                    { return message(KindSeal, nil) }

// Client is one connection. Send may run ahead of Receive (pipelining); answers arrive in the
// order of the requests.
type Client struct {
	conn     io.ReadWriteCloser
	reader   *bufio.Reader
	FrameCap uint32
	// Version is the protocol version the server chose at Hello.
	Version uint16
}

// ErrUnknown means the connection closed before an answer: the request's outcome is unknown,
// never a rejection.
var ErrUnknown = errors.New("connection closed before the answer; outcome unknown")

// Dial connects as DialRaw does and sends Hello with every version the client speaks.
func Dial(network, address string) (*Client, error) {
	return DialVersions(network, address, MinVersion, MaxVersion)
}

// DialVersions connects and sends Hello offering versions min to max.
func DialVersions(network, address string, min, max uint16) (*Client, error) {
	c, err := DialRaw(network, address)
	if err != nil {
		return nil, err
	}
	a, err := c.Do(Hello(min, max))
	if err != nil {
		c.Close()
		return nil, err
	}
	if a.Status.String() == "UnsupportedVersion" {
		c.Close()
		return nil, fmt.Errorf("hello: %v (server speaks %d to %d)", a.Status, a.MinVersion, a.MaxVersion)
	}
	if a.Status != 0 {
		c.Close()
		return nil, fmt.Errorf("hello: %v", a.Status)
	}
	if a.Version < min || a.Version > max {
		c.Close()
		return nil, fmt.Errorf("hello: server chose version %d outside %d to %d", a.Version, min, max)
	}
	c.FrameCap, c.Version = a.FrameCap, a.Version
	return c, nil
}

// DialRaw connects without Hello. The network "vsock" takes the address "cid:port" (Linux only);
// "firecracker" takes "<VMM vsock socket path>:<guest port>" (any host); any other network goes
// to net.Dial.
func DialRaw(network, address string) (*Client, error) {
	var conn io.ReadWriteCloser
	var err error
	switch network {
	case "vsock":
		conn, err = dialVsock(address)
	case "firecracker":
		conn, err = dialFirecracker(address)
	default:
		conn, err = net.Dial(network, address)
	}
	if err != nil {
		return nil, err
	}
	return &Client{conn: conn, reader: bufio.NewReader(conn)}, nil
}

func (c *Client) Close() error { return c.conn.Close() }

func (c *Client) Send(r Request) error {
	_, err := c.conn.Write(r)
	return err
}

func (c *Client) Do(r Request) (Answer, error) {
	if err := c.Send(r); err != nil {
		return Answer{}, err
	}
	return c.Receive()
}

// Receive reads the next answer. A connection closed before it returns ErrUnknown.
func (c *Client) Receive() (Answer, error) {
	var h [headerLen]byte
	if _, err := io.ReadFull(c.reader, h[:]); err != nil {
		return Answer{}, unknown(err)
	}
	body := make([]byte, binary.LittleEndian.Uint32(h[:]))
	if _, err := io.ReadFull(c.reader, body); err != nil {
		return Answer{}, unknown(err)
	}
	return decode(h[4], body)
}

// Closed reports whether the server has closed the connection.
func (c *Client) Closed() bool {
	_, err := c.reader.ReadByte()
	return err != nil
}

func unknown(err error) error {
	// Linux resets, rather than closes, a Unix socket whose peer died with requests unread.
	if errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) || errors.Is(err, syscall.ECONNRESET) {
		return ErrUnknown
	}
	return err
}

type cursor struct {
	b   []byte
	err error
}

func (c *cursor) take(n int) []byte {
	if c.err != nil || n > len(c.b) {
		c.err = errors.New("short answer")
		return make([]byte, n)
	}
	v := c.b[:n]
	c.b = c.b[n:]
	return v
}
func (c *cursor) u8() byte    { return c.take(1)[0] }
func (c *cursor) u16() uint16 { return binary.LittleEndian.Uint16(c.take(2)) }
func (c *cursor) u32() uint32 { return binary.LittleEndian.Uint32(c.take(4)) }
func (c *cursor) u64() uint64 { return binary.LittleEndian.Uint64(c.take(8)) }

func decode(kind byte, body []byte) (Answer, error) {
	c := &cursor{b: body}
	a := Answer{Kind: kind, Status: Status(c.u16())}
	if a.Status == 0 {
		switch kind &^ answerBit {
		case KindHello:
			a.Version, a.FrameCap = c.u16(), c.u32()
			c.u16()
		case KindAppend:
			a.Deduplicated = c.u8() == 1
			c.take(3)
			for n := c.u32(); n > 0 && c.err == nil; n-- {
				a.Positions = append(a.Positions, Position{c.u64(), c.u64()})
			}
		case KindRead:
			for n := c.u32(); n > 0 && c.err == nil; n-- {
				r := Record{Offset: c.u64(), Revision: c.u64()}
				sl, kl, pl := c.u16(), c.u16(), c.u32()
				r.Stream, r.Key, r.Payload = c.take(int(sl)), c.take(int(kl)), c.take(int(pl))
				if kl == 0 {
					r.Key = nil
				}
				a.Records = append(a.Records, r)
			}
		case KindWatermark:
			a.Watermark = c.u64()
		case KindStreamHead:
			present := c.u8() == 1
			c.take(7)
			h := Head{Revision: c.u64(), Offset: c.u64()}
			if present {
				a.Head = &h
			}
		}
	} else {
		switch a.Status.String() {
		case "Invalid":
			a.Reason = string(c.take(int(c.u16())))
		case "WrongRevision":
			a.Index = c.u32()
			has, actual := c.u8() == 1, c.u64()
			if has {
				a.Actual = &actual
			}
		case "KeyConflict":
			a.Index = c.u32()
		case "Corrupt":
			a.Segment, a.Position = c.u64(), c.u64()
		case "Io":
			a.IoKind = string(c.take(int(c.u16())))
		case "UnsupportedVersion":
			a.MinVersion, a.MaxVersion = c.u16(), c.u16()
		}
	}
	if c.err == nil && len(c.b) != 0 {
		c.err = errors.New("trailing bytes in answer")
	}
	return a, c.err
}
