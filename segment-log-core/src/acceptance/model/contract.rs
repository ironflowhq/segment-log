use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Bytes(pub Vec<u8>);
impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\"", self.0.escape_ascii())
    }
}
pub(super) fn bytes(text: &str) -> Bytes {
    Bytes(text.as_bytes().to_vec())
}

// Mirrors of the public types: this file must not import the crate under test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Expect {
    Empty,
    At(u64),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NewRec {
    pub stream: Bytes,
    pub key: Option<Bytes>,
    pub payload: Bytes,
    pub expected: Expect,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Sub(pub Vec<NewRec>);
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Pos {
    pub offset: u64,
    pub revision: u64,
}
impl fmt::Debug for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "offset {} revision {}", self.offset, self.revision)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Rec {
    pub pos: Pos,
    pub stream: Bytes,
    pub key: Option<Bytes>,
    pub payload: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Reject {
    WrongRevision { index: usize, actual: Option<u64> },
    KeyConflict { index: usize },
    PartialRetry,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Done {
    pub positions: Vec<Pos>,
    pub deduplicated: bool,
}
pub(super) type Answer = Result<Done, Reject>;

/// The timing is the part of R12 a caller can observe: answered while the group's flush is held,
/// or only after that flush.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Refused,
    AtClassification(Answer),
    AfterFlush(Answer),
}
impl Outcome {
    pub(super) fn rejected(&self) -> bool {
        matches!(
            self,
            Self::Refused | Self::AtClassification(Err(_)) | Self::AfterFlush(Err(_))
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct View {
    pub watermark: u64,
    pub records: Vec<Rec>,
    pub heads: BTreeMap<Bytes, Option<Pos>>,
}

pub(super) const CLOSER: &str = "closer";
pub(super) const GROUP_SUBMITS: usize = 16;
pub(super) const GROUP_RECORDS: usize = 64;
pub(super) const GROUP_BYTES: usize = 1024;

/// Every group ends with a closer, a new record on its own stream. It always appends, so the group
/// always flushes, and the driver waits for that flush with the blocking hold instead of a poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Open { closer: Sub },
    Group { submits: Vec<Sub>, closer: Sub },
}
impl Step {
    pub(super) fn submits(&self) -> &[Sub] {
        match self {
            Self::Open { .. } => &[],
            Self::Group { submits, .. } => submits,
        }
    }
    pub(super) fn closer(&self) -> &Sub {
        match self {
            Self::Open { closer } | Self::Group { closer, .. } => closer,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Script {
    steps: Vec<Step>,
    /// `without` keeps every stream, so both runs of an erasure read the same heads.
    streams: BTreeSet<Bytes>,
}
impl Script {
    pub(super) fn new() -> Self {
        let mut script = Self {
            steps: vec![],
            streams: [bytes(CLOSER)].into(),
        };
        script.open();
        script
    }
    fn next_closer(&self) -> Sub {
        let n = self.steps.len() as u64;
        Sub(vec![NewRec {
            stream: bytes(CLOSER),
            key: None,
            payload: bytes(&n.to_string()),
            expected: n.checked_sub(1).map_or(Expect::Empty, Expect::At),
        }])
    }
    pub(super) fn open(&mut self) -> &Sub {
        let closer = self.next_closer();
        self.steps.push(Step::Open { closer });
        self.steps[self.steps.len() - 1].closer()
    }
    pub(super) fn group(&mut self, submits: Vec<Sub>) -> &Sub {
        let records = || submits.iter().flat_map(|sub| &sub.0);
        assert!(records().all(|r| r.stream != bytes(CLOSER)));
        assert!(submits.len() < GROUP_SUBMITS);
        assert!(records().count() < GROUP_RECORDS);
        let size = |r: &NewRec| {
            r.stream.0.len() + r.key.as_ref().map_or(0, |k| k.0.len()) + r.payload.0.len()
        };
        assert!(records().map(size).sum::<usize>() < GROUP_BYTES);
        self.streams.extend(records().map(|r| r.stream.clone()));
        let closer = self.next_closer();
        self.steps.push(Step::Group { submits, closer });
        self.steps[self.steps.len() - 1].closer()
    }
    pub(super) fn steps(&self) -> &[Step] {
        &self.steps
    }
    pub(super) fn streams(&self) -> &BTreeSet<Bytes> {
        &self.streams
    }
    pub(super) fn max_records(&self) -> usize {
        let records = |sub: &Sub| sub.0.len();
        let step = |step: &Step| {
            step.submits().iter().map(records).sum::<usize>() + records(step.closer())
        };
        self.steps.iter().map(step).sum()
    }
    pub(super) fn without(&self, erased: &[(usize, usize)]) -> Self {
        let step = |(n, step): (usize, &Step)| match step {
            Step::Open { .. } => step.clone(),
            Step::Group { submits, closer } => Step::Group {
                submits: kept(submits, n, erased),
                closer: closer.clone(),
            },
        };
        Self {
            steps: self.steps.iter().enumerate().map(step).collect(),
            streams: self.streams.clone(),
        }
    }
}
fn kept<T: Clone>(items: &[T], step: usize, erased: &[(usize, usize)]) -> Vec<T> {
    let keep = |(i, _): &(usize, &T)| !erased.contains(&(step, *i));
    items
        .iter()
        .enumerate()
        .filter(keep)
        .map(|(_, item)| item.clone())
        .collect()
}
impl fmt::Display for Script {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (n, step) in self.steps.iter().enumerate() {
            let kind = match step {
                Step::Open { .. } => "open",
                Step::Group { .. } => "group",
            };
            writeln!(f, "step {n}: {kind}")?;
            for (i, sub) in step.submits().iter().enumerate() {
                writeln!(f, "  submit {i}: {:?}", sub.0)?;
            }
            writeln!(f, "  closer: {:?}", step.closer().0)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Observed {
    pub outcomes: Vec<Outcome>,
    pub closer: Outcome,
    pub held: View,
    /// Equals the step number while each step is one group with one flush.
    pub flushed: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Trace {
    pub steps: Vec<Observed>,
    pub end: View,
    pub flushed: usize,
}
impl Trace {
    pub(super) fn rejected(&self) -> Vec<(usize, usize)> {
        let mut rejected = vec![];
        for (n, step) in self.steps.iter().enumerate() {
            for (i, outcome) in step.outcomes.iter().enumerate() {
                if outcome.rejected() {
                    rejected.push((n, i));
                }
            }
        }
        rejected
    }
    pub(super) fn without(&self, erased: &[(usize, usize)]) -> Self {
        let step = |(n, step): (usize, &Observed)| Observed {
            outcomes: kept(&step.outcomes, n, erased),
            ..step.clone()
        };
        Self {
            steps: self.steps.iter().enumerate().map(step).collect(),
            ..self.clone()
        }
    }
    fn lines(&self) -> Vec<(String, String)> {
        fn view(lines: &mut Vec<(String, String)>, place: &str, view: &View, flushed: usize) {
            lines.push((format!("{place} finished flushes"), flushed.to_string()));
            lines.push((format!("{place} watermark"), view.watermark.to_string()));
            lines.push((format!("{place} stream heads"), format!("{:?}", view.heads)));
            lines.push((format!("{place} records"), format!("{:?}", view.records)));
        }
        let mut lines = vec![];
        for (n, step) in self.steps.iter().enumerate() {
            for (i, outcome) in step.outcomes.iter().enumerate() {
                lines.push((format!("step {n} submit {i}"), format!("{outcome:?}")));
            }
            lines.push((format!("step {n} closer"), format!("{:?}", step.closer)));
            view(
                &mut lines,
                &format!("step {n} held"),
                &step.held,
                step.flushed,
            );
        }
        view(&mut lines, "end", &self.end, self.flushed);
        lines
    }
    pub(super) fn difference(&self, other: &Self) -> Option<String> {
        if self == other {
            return None;
        }
        let (a, b) = (self.lines(), other.lines());
        let first = (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i));
        let line = |lines: &[(String, String)]| {
            let line = first.and_then(|i| lines.get(i));
            line.map_or("nothing".into(), |(place, value)| {
                format!("{place}: {value}")
            })
        };
        Some(format!("\n  {}\n  {}", line(&a), line(&b)))
    }
}
impl fmt::Display for Trace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut lines = self.lines().into_iter();
        lines.try_for_each(|(place, value)| writeln!(f, "{place}: {value}"))
    }
}

#[derive(Default)]
pub(super) struct Model {
    records: Vec<Rec>,
    durable: usize,
}

/// Only `classify` builds one, so a rejection has nothing to hand to `apply`.
struct Staged(Vec<Rec>);

enum Verdict {
    Refused,
    Rejected(Reject),
    Retry { positions: Vec<Pos>, waits: bool },
    Append(Staged),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Matches {
    Pending,
    Mixed,
    Durable,
}
/// Generator coverage only, never compared with the log. `pending` means the verdict read a record
/// of an earlier submit in the same group, which is the R12 case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Shape {
    Refused,
    KeyConflict { pending: bool },
    PartialRetry { pending: bool },
    WrongRevision { pending: bool },
    Retry(Matches),
    Appended { pending: bool },
}
impl Shape {
    pub(super) const ALL: [Self; 12] = [
        Self::Refused,
        Self::KeyConflict { pending: false },
        Self::KeyConflict { pending: true },
        Self::PartialRetry { pending: false },
        Self::PartialRetry { pending: true },
        Self::WrongRevision { pending: false },
        Self::WrongRevision { pending: true },
        Self::Retry(Matches::Pending),
        Self::Retry(Matches::Mixed),
        Self::Retry(Matches::Durable),
        Self::Appended { pending: false },
        Self::Appended { pending: true },
    ];
}
struct Classified {
    verdict: Verdict,
    shape: Shape,
}

pub(super) fn head<'a>(records: &'a [Rec], stream: &Bytes) -> Option<&'a Rec> {
    records.iter().rev().find(|r| &r.stream == stream)
}

impl Model {
    /// api.md "Order of checks" and write path step 3, in that order.
    fn classify(&self, sub: &Sub) -> Classified {
        let classified = |verdict, shape| Classified { verdict, shape };
        let pending = |r: &Rec| r.pos.offset >= self.durable as u64;

        let keys: Vec<&Bytes> = sub.0.iter().filter_map(|r| r.key.as_ref()).collect();
        let repeated = (1..keys.len()).any(|i| keys[..i].contains(&keys[i]));
        if sub.0.is_empty() || repeated {
            return classified(Verdict::Refused, Shape::Refused);
        }

        let known = |r: &NewRec| {
            let key = r.key.as_ref()?;
            self.records
                .iter()
                .find(|old| old.key.as_ref() == Some(key))
        };
        let hits: Vec<Option<&Rec>> = sub.0.iter().map(known).collect();
        for (index, (new, hit)) in sub.0.iter().zip(&hits).enumerate() {
            if let Some(old) = hit
                && (&old.stream, &old.payload) != (&new.stream, &new.payload)
            {
                let shape = Shape::KeyConflict {
                    pending: pending(old),
                };
                return classified(Verdict::Rejected(Reject::KeyConflict { index }), shape);
            }
        }
        let matched: Vec<&Rec> = hits.into_iter().flatten().collect();
        let waits = matched.iter().any(|r| pending(r));
        if matched.len() == sub.0.len() {
            let matches = match (waits, matched.iter().all(|r| pending(r))) {
                (false, _) => Matches::Durable,
                (true, true) => Matches::Pending,
                (true, false) => Matches::Mixed,
            };
            let positions = matched.iter().map(|r| r.pos).collect();
            return classified(Verdict::Retry { positions, waits }, Shape::Retry(matches));
        }
        if !matched.is_empty() {
            let shape = Shape::PartialRetry { pending: waits };
            return classified(Verdict::Rejected(Reject::PartialRetry), shape);
        }

        let mut staged: Vec<Rec> = vec![];
        let mut saw_pending = false;
        for (index, new) in sub.0.iter().enumerate() {
            let own = head(&staged, &new.stream);
            let last = own.or_else(|| head(&self.records, &new.stream));
            let read_pending = own.is_none() && last.is_some_and(pending);
            let actual = last.map(|r| r.pos.revision);
            let expected = actual.map_or(Expect::Empty, Expect::At);
            if new.expected != expected {
                let shape = Shape::WrongRevision {
                    pending: read_pending,
                };
                let reject = Reject::WrongRevision { index, actual };
                return classified(Verdict::Rejected(reject), shape);
            }
            saw_pending |= read_pending;
            staged.push(Rec {
                pos: Pos {
                    offset: (self.records.len() + staged.len()) as u64,
                    revision: actual.map_or(0, |revision| revision + 1),
                },
                stream: new.stream.clone(),
                key: new.key.clone(),
                payload: new.payload.clone(),
            });
        }
        let shape = Shape::Appended {
            pending: saw_pending,
        };
        classified(Verdict::Append(Staged(staged)), shape)
    }
    fn apply(&mut self, staged: Staged) -> Done {
        let positions = staged.0.iter().map(|r| r.pos).collect();
        self.records.extend(staged.0);
        Done {
            positions,
            deduplicated: false,
        }
    }
    pub(super) fn submit(&mut self, sub: &Sub) -> (Outcome, Shape) {
        let Classified { verdict, shape } = self.classify(sub);
        let outcome = match verdict {
            Verdict::Refused => Outcome::Refused,
            Verdict::Rejected(reject) => Outcome::AtClassification(Err(reject)),
            Verdict::Retry { positions, waits } => {
                let done = Ok(Done {
                    positions,
                    deduplicated: true,
                });
                if waits {
                    Outcome::AfterFlush(done)
                } else {
                    Outcome::AtClassification(done)
                }
            }
            Verdict::Append(staged) => Outcome::AfterFlush(Ok(self.apply(staged))),
        };
        (outcome, shape)
    }
    pub(super) fn flush(&mut self) {
        self.durable = self.records.len();
    }
    pub(super) fn written(&self) -> &[Rec] {
        &self.records
    }
    pub(super) fn durable(&self) -> &[Rec] {
        &self.records[..self.durable]
    }
    pub(super) fn view(&self, streams: &BTreeSet<Bytes>) -> View {
        let head = |stream: &Bytes| (stream.clone(), head(self.durable(), stream).map(|r| r.pos));
        View {
            watermark: self.durable as u64,
            records: self.durable().to_vec(),
            heads: streams.iter().map(head).collect(),
        }
    }
}

pub(super) fn predict(script: &Script) -> Trace {
    let streams = script.streams();
    let mut model = Model::default();
    let step = |(n, step): (usize, &Step)| {
        // Write path step 5 comes before the next group's step 2, so the group before is durable here.
        model.flush();
        let outcomes = step
            .submits()
            .iter()
            .map(|sub| model.submit(sub).0)
            .collect();
        let closer = model.submit(step.closer()).0;
        assert!(
            matches!(&closer, Outcome::AfterFlush(Ok(done)) if !done.deduplicated),
            "a closer always appends: {closer:?}"
        );
        Observed {
            outcomes,
            closer,
            held: model.view(streams),
            flushed: n,
        }
    };
    let steps = script.steps().iter().enumerate().map(step).collect();
    model.flush();
    Trace {
        steps,
        end: model.view(streams),
        flushed: script.steps().len(),
    }
}
