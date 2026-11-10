//! A request's answer, and how the answers of a durable group reach their callers
//! ([ADR 0044](../../../docs/decisions/0044-woken-callers-relay-group-acks.md)).
use super::*;
use std::thread::{self, Thread};

type Answer = Result<Committed, CommitError>;

#[derive(Default)]
struct State {
    answer: Option<Answer>,
    /// The caller's thread, from when `wait` finds no answer until someone wakes it.
    waiter: Option<Thread>,
    /// Left by whoever woke this caller: its place in the group, whose children it must wake.
    relay: Option<(Arc<Relay>, usize)>,
}

#[derive(Default)]
pub(super) struct Slot(Mutex<State>);

/// The writer's half of a request's answer. Dropped unanswered, it reports an unknown outcome.
pub(super) struct Reply {
    slot: Option<Arc<Slot>>,
}

pub struct Pending {
    slot: Arc<Slot>,
}

pub(super) fn pair() -> (Reply, Pending) {
    let slot = Arc::new(Slot::default());
    let reply = Reply {
        slot: Some(slot.clone()),
    };
    (reply, Pending { slot })
}

impl Reply {
    /// Records the answer without waking the caller. `Relay::start` wakes it.
    pub(super) fn store(mut self, answer: Answer) -> Arc<Slot> {
        let slot = self.slot.take().unwrap();
        slot.0.lock().unwrap().answer = Some(answer);
        slot
    }

    pub(super) fn send(self, answer: Answer) {
        let slot = self.store(answer);
        let waiter = slot.0.lock().unwrap().waiter.take();
        if let Some(waiter) = waiter {
            waiter.unpark();
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            Reply { slot: Some(slot) }.send(Err(CommitError::Indeterminate));
        }
    }
}

impl Pending {
    pub fn wait(self) -> Result<Committed, CommitError> {
        let mut state = self.slot.0.lock().unwrap();
        loop {
            if let Some(answer) = state.answer.take() {
                // Cleared so that a relay which reaches this place later wakes the children itself.
                state.waiter = None;
                let relay = state.relay.take();
                drop(state);
                if let Some((relay, place)) = relay {
                    relay.wake_children(Some(place));
                }
                return answer;
            }
            if state.waiter.is_none() {
                state.waiter = Some(thread::current());
            }
            drop(state);
            // `park` can return early, and an `unpark` can come first, so only the answer decides.
            thread::park();
            state = self.slot.0.lock().unwrap();
        }
    }

    #[cfg(test)]
    pub(crate) fn probe(&self) -> Option<Result<Committed, CommitError>> {
        self.slot.0.lock().unwrap().answer.take()
    }

    /// Reports whether a caller is parked in `wait` on this request.
    #[cfg(test)]
    pub(crate) fn parked(&self) -> impl Fn() -> bool + Send + 'static {
        let slot = self.slot.clone();
        move || slot.0.lock().unwrap().waiter.is_some()
    }
}

/// The answered slots of one durable group. The writer wakes the first few callers and each
/// woken caller wakes a few more, so the writer's cost does not grow with the group and the
/// wake-ups run on many threads at once.
pub(super) struct Relay(Vec<Arc<Slot>>);

impl Relay {
    /// Places form a tree. The writer takes places `0..FANOUT`, and place `p` has the
    /// children `(p + 1) * FANOUT..(p + 2) * FANOUT`.
    const FANOUT: usize = 4;

    pub(super) fn start(slots: Vec<Arc<Slot>>) {
        Arc::new(Self(slots)).wake_children(None);
    }

    fn wake_children(self: &Arc<Self>, parent: Option<usize>) {
        let first = parent.map_or(0, |place| (place + 1) * Self::FANOUT);
        for child in first..(first + Self::FANOUT).min(self.0.len()) {
            let mut state = self.0[child].0.lock().unwrap();
            match state.waiter.take() {
                Some(waiter) => {
                    state.relay = Some((self.clone(), child));
                    drop(state);
                    waiter.unpark();
                }
                // No parked caller will relay from this place, so its children are woken here.
                None => {
                    drop(state);
                    self.wake_children(Some(child));
                }
            }
        }
    }
}
