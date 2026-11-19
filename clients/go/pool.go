package segmentlog

import (
	"errors"
	"fmt"
	"sync"
	"sync/atomic"
)

// ErrPoolClosed means the pool was closed before the request was sent, so the request had no
// effect. Unlike ErrUnknown, it is never a possible append.
var ErrPoolClosed = errors.New("pool closed; request not sent")

// ErrNotSent means the request was never written: its connection failed first, or a new
// connection could not be dialed. Like ErrPoolClosed, it is never a possible append.
var ErrNotSent = errors.New("pool connection failed; request not sent")

// maxInFlight bounds the requests in flight on one pool connection to the server's read-ahead
// (protocol.md, step 3), so more in flight would only add bytes buffered in the client.
const maxInFlight = 64

// Pool is a set of pipelined connections that many goroutines share (ADR 0063). Each request goes
// to the next connection in turn. On a connection, requests queued while a write is in progress
// go out together in the next write, and each answer returns to its caller in request order
// (protocol.md, step 3). A connection holds at most 64 requests in flight; a request beyond that
// waits for an answer to free a place.
//
// When a connection fails, every request written to it and not yet answered gets ErrUnknown; the
// pool never retries one, because only a fully keyed retry is safe (api.md, retry after an
// unknown outcome). Requests queued but not yet written get ErrNotSent. The next request for that
// connection dials a new one first; if that dial fails, the request gets ErrNotSent too.
//
// The connections are shared, so the pool refuses Hello, which only opens a session. A request
// that the server finds malformed, or a Seal, ends the connection for every caller on it.
type Pool struct {
	network, address string
	slots            []*poolSlot
	next             atomic.Uint32
	closed           atomic.Bool
}

// poolSlot is one place in the pool. Its connection is replaced when it fails; its in-flight
// tokens outlive the connection, so requests still waiting on a failed one keep their places
// until they are answered.
type poolSlot struct {
	mu     sync.Mutex
	conn   *poolConn
	tokens chan struct{}
}

type poolResult struct {
	answer Answer
	err    error
}

type poolConn struct {
	c      *Client
	wake   chan struct{}
	tokens chan struct{}

	mu sync.Mutex
	// out holds requests not yet written; waiting holds a reply channel for every request queued
	// or in flight, in request order, and its last unsent entries are the requests in out. All
	// three change under mu, so their orders agree. fail closes wake under mu.
	out     []byte
	waiting []chan poolResult
	unsent  int
	failed  bool
}

// DialPool opens n connections as Dial does.
func DialPool(network, address string, n int) (*Pool, error) {
	if n < 1 {
		return nil, errors.New("a pool needs at least one connection")
	}
	clients := make([]*Client, 0, n)
	for range n {
		c, err := Dial(network, address)
		if err != nil {
			for _, c := range clients {
				c.Close()
			}
			return nil, err
		}
		clients = append(clients, c)
	}
	return newPool(network, address, clients), nil
}

func newPool(network, address string, clients []*Client) *Pool {
	p := &Pool{network: network, address: address}
	for _, c := range clients {
		s := &poolSlot{tokens: make(chan struct{}, maxInFlight)}
		s.conn = start(c, s.tokens)
		p.slots = append(p.slots, s)
	}
	return p
}

func start(c *Client, tokens chan struct{}) *poolConn {
	pc := &poolConn{c: c, wake: make(chan struct{}, 1), tokens: tokens}
	go pc.writeLoop()
	go pc.readLoop()
	return pc
}

// Do sends r on one of the pool's connections and waits for its answer. It is safe to call from
// many goroutines at once.
func (p *Pool) Do(r Request) (Answer, error) {
	res := <-p.send(r)
	return res.answer, res.err
}

func (p *Pool) send(r Request) <-chan poolResult {
	if len(r) > 4 && r[4] == KindHello {
		return failed(errors.New("a pool request cannot be Hello: a second Hello ends the connection"))
	}
	s := p.slots[(p.next.Add(1)-1)%uint32(len(p.slots))]
	s.tokens <- struct{}{}
	// A connection can fail between conn and send; then nothing was queued, so dial once more.
	for range 2 {
		pc, err := p.conn(s)
		if err != nil {
			<-s.tokens
			return failed(err)
		}
		if reply, ok := pc.send(r); ok {
			return reply
		}
	}
	<-s.tokens
	return failed(ErrNotSent)
}

func failed(err error) <-chan poolResult {
	reply := make(chan poolResult, 1)
	reply <- poolResult{err: err}
	return reply
}

// conn returns the slot's connection, dialing a new one if it has failed. The dial runs outside
// s.mu, so a server that never answers Hello cannot block Close or the slot's other callers.
func (p *Pool) conn(s *poolSlot) (*poolConn, error) {
	s.mu.Lock()
	if p.closed.Load() {
		s.mu.Unlock()
		return nil, ErrPoolClosed
	}
	if !s.conn.isFailed() {
		defer s.mu.Unlock()
		return s.conn, nil
	}
	s.mu.Unlock()
	c, err := Dial(p.network, p.address)
	if err != nil && p.closed.Load() {
		return nil, ErrPoolClosed
	}
	if err != nil {
		// %v, not %w: a Hello cut short reads as ErrUnknown, but this request was never sent.
		return nil, fmt.Errorf("%w: redial: %v", ErrNotSent, err)
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if p.closed.Load() {
		c.Close()
		return nil, ErrPoolClosed
	}
	if !s.conn.isFailed() {
		// Another caller redialed first.
		c.Close()
		return s.conn, nil
	}
	s.conn = start(c, s.tokens)
	return s.conn, nil
}

// Close closes every connection. Requests in flight get ErrUnknown, and later ones ErrPoolClosed.
func (p *Pool) Close() error {
	p.closed.Store(true)
	var first error
	for _, s := range p.slots {
		s.mu.Lock()
		err := s.conn.fail(ErrPoolClosed)
		s.mu.Unlock()
		if err != nil && first == nil {
			first = err
		}
	}
	return first
}

func (pc *poolConn) isFailed() bool {
	pc.mu.Lock()
	defer pc.mu.Unlock()
	return pc.failed
}

// send queues r, unless the connection has failed.
func (pc *poolConn) send(r Request) (<-chan poolResult, bool) {
	reply := make(chan poolResult, 1)
	pc.mu.Lock()
	defer pc.mu.Unlock()
	if pc.failed {
		return nil, false
	}
	pc.out = append(pc.out, r...)
	pc.waiting = append(pc.waiting, reply)
	pc.unsent++
	// Under mu, so that fail cannot close wake first.
	select {
	case pc.wake <- struct{}{}:
	default: // a wake is already pending; the writer takes this request with it
	}
	return reply, true
}

func (pc *poolConn) writeLoop() {
	var buf []byte
	for range pc.wake {
		pc.mu.Lock()
		if pc.failed {
			pc.mu.Unlock()
			return
		}
		buf, pc.out = pc.out, buf[:0]
		pc.unsent = 0
		pc.mu.Unlock()
		if len(buf) == 0 {
			continue
		}
		if err := pc.c.Send(buf); err != nil {
			pc.fail(ErrNotSent)
			return
		}
	}
}

func (pc *poolConn) readLoop() {
	for {
		a, err := pc.c.Receive()
		if err != nil {
			pc.fail(ErrNotSent)
			return
		}
		pc.mu.Lock()
		if len(pc.waiting) == 0 {
			// An answer nobody asked for: the stream is out of step, so no later answer can be
			// matched to its request.
			pc.mu.Unlock()
			pc.fail(ErrNotSent)
			return
		}
		reply := pc.waiting[0]
		pc.waiting = pc.waiting[1:]
		pc.mu.Unlock()
		pc.answer(reply, poolResult{answer: a})
	}
}

// answer delivers a result and frees the request's place in flight.
func (pc *poolConn) answer(reply chan poolResult, res poolResult) {
	reply <- res
	<-pc.tokens
}

// fail closes the connection once. Every request handed to the writer and not answered gets
// ErrUnknown, because some of it may have reached the server (TRANSPORT-UNKNOWN); every request
// still in out gets notSent.
func (pc *poolConn) fail(notSent error) error {
	pc.mu.Lock()
	if pc.failed {
		pc.mu.Unlock()
		return nil
	}
	pc.failed = true
	waiting, unsent := pc.waiting, pc.unsent
	pc.waiting, pc.out, pc.unsent = nil, nil, 0
	close(pc.wake)
	pc.mu.Unlock()
	err := pc.c.Close()
	for i, reply := range waiting {
		err := ErrUnknown
		if i >= len(waiting)-unsent {
			err = notSent
		}
		pc.answer(reply, poolResult{err: err})
	}
	return err
}
