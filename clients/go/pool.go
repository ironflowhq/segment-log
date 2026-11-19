package segmentlog

import (
	"errors"
	"sync"
	"sync/atomic"
)

// ErrPoolClosed means the request was never sent: the pool was closed, or the connection it
// picked had already failed. Unlike ErrUnknown, the request had no effect.
var ErrPoolClosed = errors.New("pool connection closed; request not sent")

// Pool is a set of pipelined connections that many goroutines share (ADR 0063). Each request goes
// to the next connection in turn. On a connection, requests queued while a write is in progress
// go out together in the next write, and each answer returns to its caller in request order
// (protocol.md, step 3).
//
// A connection that fails stays failed: every request in flight on it gets ErrUnknown, and later
// requests sent to it get ErrPoolClosed. Close the pool and dial a new one to recover.
type Pool struct {
	conns []*poolConn
	next  atomic.Uint32
}

type poolResult struct {
	answer Answer
	err    error
}

type poolConn struct {
	c    *Client
	wake chan struct{}

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
	return newPool(clients), nil
}

func newPool(clients []*Client) *Pool {
	p := &Pool{}
	for _, c := range clients {
		pc := &poolConn{c: c, wake: make(chan struct{}, 1)}
		p.conns = append(p.conns, pc)
		go pc.writeLoop()
		go pc.readLoop()
	}
	return p
}

// Do sends r on one of the pool's connections and waits for its answer. It is safe to call from
// many goroutines at once.
func (p *Pool) Do(r Request) (Answer, error) {
	res := <-p.send(r)
	return res.answer, res.err
}

func (p *Pool) send(r Request) <-chan poolResult {
	pc := p.conns[(p.next.Add(1)-1)%uint32(len(p.conns))]
	return pc.send(r)
}

// Close closes every connection. Requests in flight get ErrUnknown.
func (p *Pool) Close() error {
	var first error
	for _, pc := range p.conns {
		if err := pc.fail(); err != nil && first == nil {
			first = err
		}
	}
	return first
}

func (pc *poolConn) send(r Request) <-chan poolResult {
	reply := make(chan poolResult, 1)
	pc.mu.Lock()
	if pc.failed {
		pc.mu.Unlock()
		reply <- poolResult{err: ErrPoolClosed}
		return reply
	}
	pc.out = append(pc.out, r...)
	pc.waiting = append(pc.waiting, reply)
	// Under mu, so that fail cannot close wake first.
	select {
	case pc.wake <- struct{}{}:
	default: // a wake is already pending; the writer takes this request with it
	}
	pc.mu.Unlock()
	return reply
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
		reply <- poolResult{answer: a}
	}
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
		reply <- poolResult{err: ErrUnknown}
	}
	return err
}
