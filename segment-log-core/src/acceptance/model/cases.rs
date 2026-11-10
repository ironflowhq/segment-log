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
    Mixed(Vec<RecordSpec>),
    Retry {
        picks: Vec<u8>,
        expected: ExpectSpec,
    },
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
    Right,
    Empty,
    At(u8),
    Stale,
    /// What `stream_head` reports: wrong exactly when the stream has a pending record.
    DurableHead,
}

impl Erase {
    /// A subset, so a rejection that wrongly kept a key or revision still fails a kept submit after it.
    /// It holds a writer rejection when the run has one: a refused submit never reached the writer.
    pub(super) fn choose(
        &self,
        rejected: &[(usize, usize)],
        refused: &[(usize, usize)],
    ) -> Vec<(usize, usize)> {
        let masked =
            |(i, at): (usize, &(usize, usize))| self.mask[i % self.mask.len()].then_some(*at);
        let mut chosen: Vec<_> = rejected.iter().enumerate().filter_map(masked).collect();
        let writer: Vec<_> = rejected.iter().filter(|at| !refused.contains(at)).collect();
        let fallback = |len: usize| usize::from(self.fallback) % len;
        if !writer.is_empty() && chosen.iter().all(|at| refused.contains(at)) {
            chosen.push(*writer[fallback(writer.len())]);
            chosen.sort();
        } else if chosen.is_empty() && !rejected.is_empty() {
            chosen.push(rejected[fallback(rejected.len())]);
        }
        chosen
    }
}

pub(super) struct Expected {
    pub script: Script,
    pub tally: BTreeMap<Shape, u32>,
}

pub(super) fn expect(case: &Case) -> Expected {
    let mut model = Model::default();
    let mut script = Script::new();
    model.submit(script.steps()[0].closer());
    let mut memory = Memory::default();
    let mut tally = BTreeMap::new();
    for step in &case.steps {
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
    Expected { script, tally }
}

#[derive(Default)]
struct Memory {
    offered: Vec<NewRec>,
    sent: Vec<Sub>,
    fresh_keys: u32,
}
struct Draft {
    stream: Bytes,
    key: Option<Bytes>,
    payload: Bytes,
    expected: ExpectSpec,
}

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
        // A stream with no records has no revision to be behind, and `At(0)` is wrong there too.
        ExpectSpec::Stale if next == 0 => Expect::At(0),
        ExpectSpec::Stale => at(next - 1),
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
