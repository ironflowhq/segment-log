//! Generated cases. A case is plain data that names no model state, so each field keeps its meaning while it shrinks.
//! `expect` resolves a case against the model alone. This file names nothing of the log under test.
use super::contract::*;
use proptest::{collection::vec, prelude::*};
use std::collections::BTreeMap;

// One name is a prefix of another, and one payload is empty, so a comparison of lengths or prefixes shows.
const STREAMS: [&str; 3] = ["s", "ss", "t"];
const PAYLOADS: [&str; 3] = ["", "p", "pp"];

#[derive(Clone, Debug)]
pub(super) struct Case {
    pub steps: Vec<StepSpec>,
    pub erase: Erase,
}
/// Which rejected submits the erasure property removes.
#[derive(Clone, Debug)]
pub(super) struct Erase {
    mask: Vec<bool>,
    fallback: u8,
}
#[derive(Clone, Debug)]
pub(super) enum StepSpec {
    Group(Vec<SubmitSpec>),
    Reopen,
}
#[derive(Clone, Debug)]
pub(super) enum SubmitSpec {
    /// Records chosen one by one. No record gives the empty submit.
    Mixed(Vec<RecordSpec>),
    /// Every record repeats an accepted keyed record, so the submit is a fully matching retry
    /// that can take a subset, reorder and combine originals. A pick of a record already taken adds nothing.
    Retry {
        picks: Vec<u8>,
        expected: ExpectSpec,
    },
    /// An earlier submit again, unchanged, as a caller replays it after an unknown outcome.
    Replay(u8),
}
#[derive(Clone, Debug)]
pub(super) enum RecordSpec {
    Fresh {
        stream: u8,
        keyed: bool,
        payload: u8,
        expected: ExpectSpec,
    },
    /// Reuses the key of an offered record, whether the log accepted it or not.
    Echo {
        pick: u8,
        twist: Twist,
        expected: ExpectSpec,
    },
}
#[derive(Clone, Copy, Debug)]
pub(super) enum Twist {
    Exact,
    OtherStream,
    OtherPayload,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum ExpectSpec {
    /// Passes the revision check when the records before it in the submit pass theirs.
    Right,
    Empty,
    At(u8),
    /// One revision behind `Right`.
    Stale,
    /// What `stream_head` reports. It is wrong when the stream has a pending record,
    /// or an earlier record of the same submit.
    DurableHead,
}

impl Erase {
    /// A subset of `rejected` with at least one member when there is one.
    /// The rejected submits that stay show whether an erased one had left something behind.
    pub(super) fn choose(&self, rejected: &[(usize, usize)]) -> Vec<(usize, usize)> {
        let masked =
            |(i, at): (usize, &(usize, usize))| self.mask[i % self.mask.len()].then_some(*at);
        let chosen: Vec<_> = rejected.iter().enumerate().filter_map(masked).collect();
        if chosen.is_empty() && !rejected.is_empty() {
            return vec![rejected[usize::from(self.fallback) % rejected.len()]];
        }
        chosen
    }
}

pub(super) struct Expected {
    pub script: Script,
    pub trace: Trace,
    pub tally: BTreeMap<Shape, u32>,
}

/// Resolves the case into concrete submits and asks the model what a log must show for them.
pub(super) fn expect(case: &Case) -> Expected {
    let mut model = Model::default();
    let mut script = Script::new();
    model.submit(script.steps()[0].closer());
    let mut memory = Memory::default();
    let mut tally = BTreeMap::new();
    for step in &case.steps {
        // This model picks the inputs, so it takes each step in the order of `predict`.
        model.flush();
        let closer = match step {
            StepSpec::Reopen => script.open(),
            StepSpec::Group(specs) => {
                let mut submits = vec![];
                for spec in specs {
                    let sub = memory.resolve(spec, &model);
                    let (_, shape) = model.submit(&sub);
                    *tally.entry(shape).or_default() += 1;
                    memory.remember(&sub, shape);
                    submits.push(sub);
                }
                script.group(submits)
            }
        };
        model.submit(closer);
    }
    let trace = predict(&script);
    Expected {
        script,
        trace,
        tally,
    }
}

/// What a caller remembers of its own submits. The log may have rejected any of them.
#[derive(Default)]
struct Memory {
    /// The first offer of each key.
    offered: Vec<NewRec>,
    sent: Vec<Sub>,
    fresh_keys: u32,
}
/// A record before the model resolves its expectation.
struct Draft {
    stream: Bytes,
    key: Option<Bytes>,
    payload: Bytes,
    expected: ExpectSpec,
}

/// Counts back from the newest item, so most picks land in the current group, where the records are pending.
fn recent<T>(items: &[T], pick: u8) -> Option<&T> {
    let back = usize::from(pick) % items.len().max(1);
    items.iter().rev().nth(back)
}
fn name(pool: &[&str], index: usize) -> Bytes {
    bytes(pool[index % pool.len()])
}
fn other(pool: &[&str], now: &Bytes) -> Bytes {
    let at = pool.iter().position(|name| &bytes(name) == now);
    name(pool, at.map_or(0, |at| at + 1))
}

impl Memory {
    fn remember(&mut self, sub: &Sub, shape: Shape) {
        // A replay of a submit that `append` refused is refused again, and it would teach nothing.
        if shape == Shape::Refused {
            return;
        }
        for record in &sub.0 {
            let known = |old: &NewRec| old.key == record.key;
            if record.key.is_some() && !self.offered.iter().any(known) {
                self.offered.push(record.clone());
            }
        }
        self.sent.push(sub.clone());
    }
    fn fresh_key(&mut self) -> Bytes {
        self.fresh_keys += 1;
        bytes(&format!("k{}", self.fresh_keys))
    }
    /// Stands in where a spec picks an earlier record and there is none yet.
    fn fresh(&mut self, pick: u8, expected: ExpectSpec) -> Draft {
        Draft {
            stream: name(&STREAMS, pick.into()),
            key: Some(self.fresh_key()),
            payload: name(&PAYLOADS, 0),
            expected,
        }
    }
    fn resolve(&mut self, spec: &SubmitSpec, model: &Model) -> Sub {
        let drafts: Vec<Draft> = match spec {
            SubmitSpec::Mixed(specs) => specs.iter().map(|spec| self.record(spec)).collect(),
            SubmitSpec::Replay(pick) => match recent(&self.sent, *pick) {
                Some(sub) => return sub.clone(),
                None => vec![self.fresh(*pick, ExpectSpec::Right)],
            },
            SubmitSpec::Retry { picks, expected } => {
                let keyed: Vec<&Rec> = model.written().iter().filter(|r| r.key.is_some()).collect();
                let mut drafts: Vec<Draft> = vec![];
                for pick in picks {
                    match recent(&keyed, *pick) {
                        None => drafts.push(self.fresh(*pick, *expected)),
                        Some(old) if drafts.iter().any(|draft| draft.key == old.key) => {}
                        Some(old) => drafts.push(Draft {
                            stream: old.stream.clone(),
                            key: old.key.clone(),
                            payload: old.payload.clone(),
                            expected: *expected,
                        }),
                    }
                }
                drafts
            }
        };
        let mut records = vec![];
        for draft in drafts {
            let expected = expectation(draft.expected, model, &draft.stream, &records);
            records.push(NewRec {
                stream: draft.stream,
                key: draft.key,
                payload: draft.payload,
                expected,
            });
        }
        Sub(records)
    }
    fn record(&mut self, spec: &RecordSpec) -> Draft {
        match *spec {
            RecordSpec::Fresh {
                stream,
                keyed,
                payload,
                expected,
            } => Draft {
                stream: name(&STREAMS, stream.into()),
                key: keyed.then(|| self.fresh_key()),
                payload: name(&PAYLOADS, payload.into()),
                expected,
            },
            RecordSpec::Echo {
                pick,
                twist,
                expected,
            } => match recent(&self.offered, pick) {
                None => self.fresh(pick, expected),
                Some(old) => {
                    let (stream, payload) = match twist {
                        Twist::Exact => (old.stream.clone(), old.payload.clone()),
                        Twist::OtherStream => (other(&STREAMS, &old.stream), old.payload.clone()),
                        Twist::OtherPayload => (old.stream.clone(), other(&PAYLOADS, &old.payload)),
                    };
                    Draft {
                        stream,
                        key: old.key.clone(),
                        payload,
                        expected,
                    }
                }
            },
        }
    }
}

fn expectation(spec: ExpectSpec, model: &Model, stream: &Bytes, earlier: &[NewRec]) -> Expect {
    let own = earlier.iter().filter(|r| &r.stream == stream).count() as u64;
    let next = head(model.written(), stream).map_or(0, |r| r.pos.revision + 1) + own;
    let at = |next: u64| next.checked_sub(1).map_or(Expect::Empty, Expect::At);
    match spec {
        ExpectSpec::Right => at(next),
        ExpectSpec::Stale => at(next.saturating_sub(1)),
        ExpectSpec::DurableHead => {
            head(model.durable(), stream).map_or(Expect::Empty, |r| Expect::At(r.pos.revision))
        }
        ExpectSpec::Empty => Expect::Empty,
        ExpectSpec::At(revision) => Expect::At(revision.into()),
    }
}

pub(super) fn case() -> impl Strategy<Value = Case> {
    let erase = (vec(any::<bool>(), 1..=8), any::<u8>());
    let erase = erase.prop_map(|(mask, fallback)| Erase { mask, fallback });
    (vec(step(), 1..=12), erase).prop_map(|(steps, erase)| Case { steps, erase })
}
fn step() -> impl Strategy<Value = StepSpec> {
    prop_oneof![
        9 => vec(submit(), 0..=6).prop_map(StepSpec::Group),
        1 => Just(StepSpec::Reopen),
    ]
}
fn submit() -> impl Strategy<Value = SubmitSpec> {
    let retry = (vec(pick(), 1..=4), expected());
    prop_oneof![
        24 => vec(record(), 1..=4).prop_map(SubmitSpec::Mixed),
        6 => retry.prop_map(|(picks, expected)| SubmitSpec::Retry { picks, expected }),
        3 => pick().prop_map(SubmitSpec::Replay),
        1 => Just(SubmitSpec::Mixed(vec![])),
    ]
}
fn record() -> impl Strategy<Value = RecordSpec> {
    let fresh = (0..3u8, proptest::bool::weighted(0.7), 0..3u8, expected());
    let twist = prop_oneof![
        3 => Just(Twist::Exact),
        1 => Just(Twist::OtherStream),
        1 => Just(Twist::OtherPayload),
    ];
    let echo = (pick(), twist, expected());
    prop_oneof![
        3 => fresh.prop_map(|(stream, keyed, payload, expected)| {
            RecordSpec::Fresh { stream, keyed, payload, expected }
        }),
        1 => echo.prop_map(|(pick, twist, expected)| RecordSpec::Echo { pick, twist, expected }),
    ]
}
/// Mostly a recent item. Now and then any item, so that old records in earlier frames get picked too.
fn pick() -> impl Strategy<Value = u8> {
    prop_oneof![
        4 => 0..10u8,
        1 => any::<u8>(),
    ]
}
fn expected() -> impl Strategy<Value = ExpectSpec> {
    prop_oneof![
        20 => Just(ExpectSpec::Right),
        1 => Just(ExpectSpec::Empty),
        1 => (0..4u8).prop_map(ExpectSpec::At),
        1 => Just(ExpectSpec::Stale),
        3 => Just(ExpectSpec::DurableHead),
    ]
}
