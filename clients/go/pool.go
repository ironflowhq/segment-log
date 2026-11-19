package segmentlog

import (
	"errors"
	"sync"
	"sync/atomic"
)

// ErrPoolClosed means the pool was closed before the request was sent, so the request had no
// effect. Unlike ErrUnknown, it is never a possible append.
var ErrPoolClosed = errors.New("pool closed; request not sent")

// maxInFlight bounds the requests in flight on one pool connection. The server reads no more than
// 64 requests ahead of its answers (PIPELINE_DEPTH, ADR 0063), so more in flight would only add
// bytes buffered in the client.
const maxInFlight = 64

// Pool is a set of pipelined connections that many goroutines share (ADR 0063). Each request goes
// to the next connection in turn. On a connection, requests queued while a write is in progress
// go out together in the next write, and each answer returns to its caller in request order
// (protocol.md, step 3). A connection holds at most 64 requests in flight; a request beyond that
// waits for an answer to free a place.
//
// When a connection fails, every request in flight on it gets ErrUnknown; the pool never retries
// one, because only a fully keyed retry is safe (api.md, retry after an unknown outcome). The next
// request for that connection dials a new one first. If that dial fails, the request gets its
// error and was not sent.
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
	// or in flight, in request order. Both change under mu, so their orders agree. fail closes
	// wake under mu.
	out     []byte
	waiting []chan poolResult
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
	s := p.slots[(p.next.Add(1)-1)%uint32(len(p.slots))]
	s.tokens <- struct{}{}
	for {
		pc, err := p.conn(s)
		if err != nil {
			<-s.tokens
			reply := make(chan poolResult, 1)
			reply <- poolResult{err: err}
			return reply
		}
		// A connection can fail between conn and send; then nothing was queued, so dial again.
		if reply, ok := pc.send(r); ok {
			return reply
		}
	}
}

// conn returns the slot's connection, dialing a new one if it has failed.
func (p *Pool) conn(s *poolSlot) (*poolConn, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if p.closed.Load() {
		return nil, ErrPoolClosed
	}
	if !s.conn.isFailed() {
		return s.conn, nil
	}
	c, err := Dial(p.network, p.address)
	if err != nil {
		return nil, err
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
		err := s.conn.fail()
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
		pc.mu.Unlock()
		if len(buf) == 0 {
			continue
		}
		if err := pc.c.Send(buf); err != nil {
			pc.fail()
			return
		}
	}
}

func (pc *poolConn) readLoop() {
	for {
		a, err := pc.c.Receive()
		if err != nil {
			pc.fail()
			return
		}
		pc.mu.Lock()
		if len(pc.waiting) == 0 {
			// An answer nobody asked for: the stream is out of step, so no later answer can be
			// matched to its request.
			pc.mu.Unlock()
			pc.fail()
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

// fail closes the connection once and resolves every request still waiting with ErrUnknown: some
// of it may have reached the server (TRANSPORT-UNKNOWN).
func (pc *poolConn) fail() error {
	pc.mu.Lock()
	if pc.failed {
		pc.mu.Unlock()
		return nil
	}
	pc.failed = true
	waiting := pc.waiting
	pc.waiting, pc.out = nil, nil
	close(pc.wake)
	pc.mu.Unlock()
	err := pc.c.Close()
	for _, reply := range waiting {
		pc.answer(reply, poolResult{err: ErrUnknown})
	}
	return err
}
