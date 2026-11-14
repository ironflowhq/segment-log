//! The connections of one server process and the decision to stop it (ADR 0058).

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Exit status after a seal or a stop signal (ADR 0058).
pub const EXIT_STOPPED: u8 = 0;

/// Exit status after a poisoned writer (ADR 0058). The supervisor restarts the server, and the new
/// open recovers the log.
pub const EXIT_POISONED: u8 = 2;

type CloseReads = Box<dyn Fn() + Send>;

pub struct Server {
    state: Mutex<State>,
    changed: Condvar,
    max_connections: usize,
}

struct State {
    /// For each open connection, a way to end its reads, so its reader stops taking requests.
    connections: HashMap<u64, CloseReads>,
    next: u64,
    exit: Option<u8>,
}

impl Server {
    pub fn new(max_connections: usize) -> Arc<Server> {
        Arc::new(Server {
            state: Mutex::new(State {
                connections: HashMap::new(),
                next: 0,
                exit: None,
            }),
            changed: Condvar::new(),
            max_connections,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding this lock leaves only plain data behind, so keep going.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A slot for a new connection, or `None` when the server is full or stopping.
    pub fn admit(self: &Arc<Self>, close_reads: CloseReads) -> Option<Slot> {
        let mut state = self.lock();
        if state.exit.is_some() || state.connections.len() >= self.max_connections {
            return None;
        }
        let id = state.next;
        state.next += 1;
        state.connections.insert(id, close_reads);
        Some(Slot {
            server: Arc::clone(self),
            id,
        })
    }

    /// Stops the server with `code`. The first call decides the code. Every connection stops
    /// reading requests; its writer still answers the ones already read.
    pub fn stop(&self, code: u8) {
        let mut state = self.lock();
        if state.exit.is_none() {
            state.exit = Some(code);
            state.connections.values().for_each(|close| close());
            self.changed.notify_all();
        }
    }

    pub fn wait_for_stop(&self) -> u8 {
        let mut state = self.lock();
        loop {
            if let Some(code) = state.exit {
                return code;
            }
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Waits until every connection has finished, or until `limit` passes. The limit exists
    /// because a client that never reads keeps its writer blocked forever.
    pub fn wait_until_idle(&self, limit: Duration) {
        let deadline = Instant::now() + limit;
        let mut state = self.lock();
        while !state.connections.is_empty() {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

/// One connection's place in the server, released when both of its threads have dropped it.
pub struct Slot {
    pub server: Arc<Server>,
    id: u64,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut state = self.server.lock();
        state.connections.remove(&self.id);
        self.server.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn the_cap_and_a_stop_refuse_new_connections() {
        let server = Server::new(1);
        let first = server.admit(Box::new(|| {})).unwrap();
        assert!(server.admit(Box::new(|| {})).is_none());
        drop(first);
        let _second = server.admit(Box::new(|| {})).unwrap();
        server.stop(EXIT_POISONED);
        drop(_second);
        assert!(server.admit(Box::new(|| {})).is_none());
    }

    #[test]
    fn stop_closes_every_connections_reads_once_and_keeps_the_first_code() {
        let server = Server::new(4);
        let closed = Arc::new(AtomicUsize::new(0));
        let slots: Vec<Slot> = (0..3)
            .map(|_| {
                let closed = Arc::clone(&closed);
                server
                    .admit(Box::new(move || {
                        closed.fetch_add(1, Ordering::SeqCst);
                    }))
                    .unwrap()
            })
            .collect();
        server.stop(EXIT_POISONED);
        server.stop(0);
        assert_eq!(closed.load(Ordering::SeqCst), 3);
        assert_eq!(server.wait_for_stop(), EXIT_POISONED);
        let waiter = {
            let server = Arc::clone(&server);
            std::thread::spawn(move || server.wait_until_idle(Duration::from_secs(10)))
        };
        drop(slots);
        waiter.join().unwrap();
    }

    #[test]
    fn waiting_for_idle_gives_up_at_the_limit() {
        let server = Server::new(1);
        let _slot = server.admit(Box::new(|| {})).unwrap();
        let start = Instant::now();
        server.wait_until_idle(Duration::from_millis(50));
        assert!(start.elapsed() >= Duration::from_millis(50));
    }
}
