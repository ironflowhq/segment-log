//! A case resolves with the model alone into a script and the trace the contract requires. The log
//! then runs the script. The model and the log never run in turns, so a wrong model cannot steer the log.
mod cases;
mod contract;
mod drive;

use super::support::deadline::Deadline;
use super::support::fixtures::options;
use cases::{Case, expect};
use contract::{
    Expect::{At, Empty},
    *,
};
use drive::drive;
use proptest::{
    strategy::{Strategy, ValueTree},
    test_runner::{
        Config, FileFailurePersistence, RngAlgorithm, RngSeed, TestCaseError, TestError, TestRunner,
    },
};
use std::{
    collections::BTreeMap,
    io::{self, Write},
    time::Duration,
};

const SEED: u64 = 2000;
const CASE_DEADLINE: Duration = Duration::from_secs(60);

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name).map_or(default, |v| v.parse().expect("numeric model setting"))
}

fn config(seed: u64, cases: u32) -> Config {
    Config {
        cases,
        rng_seed: RngSeed::Fixed(seed),
        rng_algorithm: RngAlgorithm::ChaCha,
        source_file: Some(file!()),
        failure_persistence: Some(Box::new(FileFailurePersistence::SourceParallel(
            "proptest-regressions",
        ))),
        // A group that never flushes costs the 5 s hold watchdog for each shrink step.
        max_shrink_time: 60_000,
        max_shrink_iters: 4096,
        ..Config::default()
    }
}

/// Seeds in `proptest-regressions/acceptance/model.txt` run first.
fn check(
    case_id: &'static str,
    default_cases: u32,
    property: fn(&Case) -> Result<(), TestCaseError>,
) {
    let seed = setting("SEGMENT_LOG_MODEL_SEED", SEED);
    let cases = u32::try_from(setting("SEGMENT_LOG_MODEL_CASES", default_cases.into()))
        .expect("case count fits u32");
    let mut runner = TestRunner::new(config(seed, cases));
    let run = |case: Case| {
        // Every hold and answer wait has its own deadline, but dropping the log joins the writer without one.
        // The deadline aborts, which ends shrinking, so it prints the replay values before it does.
        // It writes to stderr directly: the test's captured output is lost in the abort.
        let hung = case.clone();
        let _deadline = Deadline::new(CASE_DEADLINE, move || {
            let report = format!(
                "cases={case_id} seed={seed} count={cases}\na case ran longer than {CASE_DEADLINE:?}\n\
                 case: {hung:#?}\nscript:\n{}\nreplay: SEGMENT_LOG_MODEL_SEED={seed} \
                 SEGMENT_LOG_MODEL_CASES={cases} cargo test -p segment-log-core --lib acceptance::model\n",
                expect(&hung).script
            );
            let _ = io::stderr().write_all(report.as_bytes());
        });
        property(&case)
    };
    match runner.run(&cases::case(), run) {
        Ok(()) => {}
        Err(TestError::Abort(reason)) => {
            panic!("cases={case_id} seed={seed} count={cases}\n{reason}")
        }
        Err(TestError::Fail(reason, case)) => panic!(
            "cases={case_id} seed={seed} count={cases}\n{reason}\nminimal case: {case:#?}\nscript:\n{}\n\
             replay: SEGMENT_LOG_MODEL_SEED={seed} SEGMENT_LOG_MODEL_CASES={cases} \
             cargo test -p segment-log-core --lib acceptance::model",
            expect(&case).script
        ),
    }
}

fn equal(what: [&str; 2], a: &Trace, b: &Trace) -> Result<(), String> {
    let [first, second] = what;
    a.difference(b).map_or(Ok(()), |difference| {
        Err(format!(
            "first difference, {first} and then {second}:{difference}\n{first}:\n{a}\n{second}:\n{b}"
        ))
    })
}

fn log_agrees(case: &Case) -> Result<(), TestCaseError> {
    let script = expect(case).script;
    let observed = drive(&script, &options());
    equal(["the model", "the log"], &predict(&script), &observed).map_err(TestCaseError::fail)
}

fn rejections_erase(case: &Case) -> Result<(), TestCaseError> {
    // The model only picks the inputs. The log's own answers decide what is erased and what must stay equal.
    let script = expect(case).script;
    let full = drive(&script, &options());
    let erased = case.erase.choose(&full.rejected(), &full.refused());
    if erased.is_empty() {
        return Ok(());
    }
    let kept = drive(&script.without(&erased), &options());
    let what = format!(
        "the full run less the answers to the erased submits {erased:?}, as (step, submit)"
    );
    equal(
        [&what, "the run without those submits"],
        &full.without(&erased),
        &kept,
    )
    .map_err(TestCaseError::fail)
}

/// The M1 trace oracle classifies submits with its own sequential rules. Fed one submit at a time,
/// with a flush after each, the model must give the same answer, so the two copies of the contract
/// cannot drift apart (issue 53).
fn oracle_agrees(case: &Case) -> Result<(), TestCaseError> {
    use super::support::oracle;
    let script = expect(case).script;
    let mut model = Model::default();
    let mut history = Vec::new();
    for step in script.steps() {
        for sub in step.submits().iter().chain([step.closer()]) {
            let inputs: Vec<oracle::Input> = sub
                .0
                .iter()
                .map(|r| oracle::Input {
                    stream: r.stream.0.clone(),
                    key: r.key.as_ref().map(|k| k.0.clone()),
                    payload: r.payload.0.clone(),
                    expected: match r.expected {
                        Empty => None,
                        At(n) => Some(n),
                    },
                    spare: 0,
                })
                .collect();
            let predicted = oracle::predict(&inputs, &mut history);
            let (outcome, _) = model.submit(sub);
            model.flush();
            let answer = match outcome {
                Outcome::Refused => oracle::Answer::Refused("Invalid".into()),
                Outcome::AtClassification(answer) | Outcome::AfterFlush(answer) => match answer {
                    Ok(done) => oracle::Answer::Ack(
                        done.deduplicated,
                        done.positions
                            .iter()
                            .map(|p| (p.offset, p.revision))
                            .collect(),
                    ),
                    Err(Reject::KeyConflict { index }) => {
                        oracle::Answer::Refused(format!("KeyConflict {{ index: {index} }}"))
                    }
                    Err(Reject::PartialRetry) => oracle::Answer::Refused("PartialRetry".into()),
                    Err(Reject::WrongRevision { index, actual }) => oracle::Answer::Refused(
                        format!("WrongRevision {{ index: {index}, actual: {actual:?} }}"),
                    ),
                },
            };
            if predicted != answer {
                return Err(TestCaseError::fail(format!(
                    "submit {sub:?}\nthe oracle: {predicted:?}\nthe model: {answer:?}\nscript:\n{script}"
                )));
            }
        }
    }
    Ok(())
}

/// Given random groups of submits, resolved by the model alone
/// When each submit goes to the M1 trace oracle and to the model, with a flush after every submit
/// Then both give the same answer to every submit
///
/// Cases: ORACLE-MODEL
#[test]
fn the_trace_oracle_agrees_with_the_model() {
    check("ORACLE-MODEL", 512, oracle_agrees);
}

/// Given random groups of submits, each queued behind a held flush, and random reopens
/// When the same submits go to the model and to a log on simulated storage
/// Then every answer, its timing against its group's flush, and every durable view are equal
/// And every group here has a new record, so the group that skips its flush stays with the
/// directed group-formation scenarios
///
/// Cases: MODEL
#[test]
fn the_log_agrees_with_the_model() {
    check("MODEL", 256, log_agrees);
}

/// Given a random run, and the same run without some of the submits that the log rejected
/// When both run on fresh logs
/// Then every other submit gets the same answer at the same time, and every durable view is equal
///
/// Cases: REV-NO-OFFSET
#[test]
fn erasing_rejected_submits_changes_nothing() {
    check("REV-NO-OFFSET", 256, rejections_erase);
}

/// Given the model property with a larger case budget
/// When the scheduled Linux and macOS job runs the ignored test
/// Then a failure leaves its seed in the uploaded regression file
///
/// Cases: MODEL
#[test]
#[ignore = "long run"]
fn the_log_agrees_with_the_model_long_run() {
    check("MODEL", 20_000, log_agrees);
}

/// Given the erasure property with a larger case budget
/// When the scheduled Linux and macOS job runs the ignored test
/// Then a failure leaves its seed in the uploaded regression file
///
/// Cases: REV-NO-OFFSET
#[test]
#[ignore = "long run"]
fn erasing_rejected_submits_changes_nothing_long_run() {
    check("REV-NO-OFFSET", 20_000, rejections_erase);
}

/// Given 2,000 generated cases at a fixed seed, resolved by the model alone
/// When their verdicts are counted by shape
/// Then each rejection and each append occurs at least 50 times with and without a pending record behind it
/// And fully matching retries occur at least 50 times each against pending, durable and mixed matches
///
/// Cases: MODEL
#[test]
fn generated_cases_reach_every_shape() {
    let mut runner = TestRunner::new(Config {
        failure_persistence: None,
        ..config(SEED, 0)
    });
    let mut tally: BTreeMap<Shape, u32> = BTreeMap::new();
    for _ in 0..2000 {
        let case = cases::case().new_tree(&mut runner).unwrap().current();
        for (shape, count) in expect(&case).tally {
            *tally.entry(shape).or_default() += count;
        }
    }
    let total: u32 = tally.values().sum();
    eprintln!("shapes of {total} submits in 2000 cases:");
    for shape in Shape::ALL {
        let count = tally.get(&shape).copied().unwrap_or(0);
        eprintln!("  {count:>6} {shape:?}");
        assert!(count >= 50, "{shape:?} occurred {count} times");
    }
}

fn record(stream: &str, key: Option<&str>, payload: &str, expected: Expect) -> NewRec {
    NewRec {
        stream: bytes(stream),
        key: key.map(bytes),
        payload: bytes(payload),
        expected,
    }
}
fn sub<const N: usize>(records: [NewRec; N]) -> Sub {
    Sub(records.into())
}
fn done(positions: &[(u64, u64)], deduplicated: bool) -> Answer {
    let position = |&(offset, revision)| Pos { offset, revision };
    Ok(Done {
        positions: positions.iter().map(position).collect(),
        deduplicated,
    })
}
fn appended(positions: &[(u64, u64)]) -> Outcome {
    Outcome::AfterFlush(done(positions, false))
}
fn rejected(reject: Reject) -> Outcome {
    Outcome::AtClassification(Err(reject))
}
/// Given the model alone
/// When it classifies the examples that the revision, key and retry cases spell out
/// Then each answer and its timing has the value that the contract states
///
/// Cases: MODEL
#[test]
fn the_model_follows_the_contract_examples() {
    fn example(model: &mut Model, what: &str, records: &[NewRec], expected: Outcome) {
        assert_eq!(model.submit(&Sub(records.to_vec())).0, expected, "{what}");
    }
    let wrong = |index, actual| rejected(Reject::WrongRevision { index, actual });
    let conflict = |index| rejected(Reject::KeyConflict { index });
    let partial = || rejected(Reject::PartialRetry);
    let waits = |positions| Outcome::AfterFlush(done(positions, true));
    let a = |expected| record("a", None, "", expected);
    let k1 = |expected| record("b", Some("k1"), "one", expected);
    let k2 = |expected| record("b", Some("k2"), "two", expected);
    let k3 = |expected| record("c", Some("k3"), "three", expected);
    let k4 = || record("d", Some("k4"), "four", Empty);
    let other = |stream, key, payload, expected| record(stream, Some(key), payload, expected);
    let mixed = [
        record("e", Some("k5"), "five", Empty),
        record("e", None, "six", At(0)),
    ];

    let m = &mut Model::default();
    example(
        m,
        "`At` on a stream with no records",
        &[a(At(0))],
        wrong(0, None),
    );
    example(
        m,
        "`Empty` on a stream with no records",
        &[a(Empty)],
        appended(&[(0, 0)]),
    );
    example(
        m,
        "`Empty` at revision zero",
        &[a(Empty)],
        wrong(0, Some(0)),
    );
    example(
        m,
        "`At(0)` at revision zero",
        &[a(At(0))],
        appended(&[(1, 1)]),
    );
    example(
        m,
        "a second record that ignores the first",
        &[k1(Empty), k2(Empty)],
        wrong(1, Some(0)),
    );
    example(
        m,
        "successive records in one submit",
        &[k1(Empty), k2(At(0))],
        appended(&[(2, 0), (3, 1)]),
    );
    let repeated = [other("c", "k9", "x", Empty), other("d", "k9", "y", Empty)];
    example(
        m,
        "a repeated key on other streams and payloads",
        &repeated,
        Outcome::Refused,
    );
    example(m, "an empty submit", &[], Outcome::Refused);
    let conflicts = [
        k3(Empty),
        other("b", "k1", "changed", At(9)),
        other("z", "k2", "two", Empty),
    ];
    example(
        m,
        "the first of two conflicts, before revisions",
        &conflicts,
        conflict(1),
    );
    example(
        m,
        "the same key and payload on another stream",
        &[other("z", "k1", "one", Empty)],
        conflict(0),
    );
    example(
        m,
        "a pending match, then a fresh key",
        &[k1(Empty), k3(Empty)],
        partial(),
    );
    example(
        m,
        "an unkeyed record, then a pending match",
        &[record("c", None, "three", Empty), k2(At(0))],
        partial(),
    );
    let late_conflict = [k1(Empty), k3(Empty), other("b", "k2", "no", At(0))];
    example(
        m,
        "a conflict after a match and a fresh key",
        &late_conflict,
        conflict(2),
    );
    example(
        m,
        "a key that only rejected submits offered",
        &[k3(Empty)],
        appended(&[(4, 0)]),
    );
    example(
        m,
        "a reordered retry of pending originals, stale",
        &[k3(At(7)), k1(At(7))],
        waits(&[(4, 0), (2, 0)]),
    );
    example(
        m,
        "a subset retry of a pending original",
        &[k2(Empty)],
        waits(&[(3, 1)]),
    );
    m.flush();
    let at_once = Outcome::AtClassification(done(&[(4, 0), (2, 0)], true));
    example(
        m,
        "the same retry of durable originals",
        &[k3(At(7)), k1(At(7))],
        at_once,
    );
    example(
        m,
        "a new record after the flush",
        &[k4()],
        appended(&[(5, 0)]),
    );
    example(
        m,
        "a retry of a pending and a durable original",
        &[k4(), k2(Empty)],
        waits(&[(5, 0), (3, 1)]),
    );
    example(
        m,
        "a keyed and an unkeyed record",
        &mixed,
        appended(&[(6, 0), (7, 1)]),
    );
    example(m, "the unchanged replay of that submit", &mixed, partial());
    example(
        m,
        "the unchanged replay of an unkeyed submit",
        &[a(At(0))],
        wrong(0, Some(1)),
    );

    let view = m.view(&["a", "b", "z"].map(bytes).into());
    let head = |offset, revision| Some(Pos { offset, revision });
    let heads = [
        (bytes("a"), head(1, 1)),
        (bytes("b"), head(3, 1)),
        (bytes("z"), None),
    ];
    assert_eq!(
        (view.watermark, view.records.len()),
        (5, 5),
        "the durable view stops at the flush"
    );
    assert_eq!(
        view.heads,
        heads.into(),
        "stream heads show durable records only"
    );
}

#[test]
fn the_model_imports_nothing_from_the_crate() {
    for source in [
        include_str!("model/contract.rs"),
        include_str!("model/cases.rs"),
    ] {
        for path in ["crate::", "super::super", "super::drive", "drive::"] {
            assert!(!source.contains(path), "the model names {path}");
        }
    }
}

/// Given a log whose first group flush is held
/// When two originals on one stream, their exact, subset and regrouped retries, a conflict with a wrong
/// revision, a partial retry and a stale revision enter the next group
/// Then the retries and the originals have no answer while that group's flush is held, and the three
/// rejections have theirs, with the pending revision as `actual`
/// And after the flush the retries return the original positions in request order
/// When later groups retry durable and mixed matches, also after a reopen
/// Then an all-durable retry answers before its group's flush and a mixed one after it
///
/// Cases: MODEL
#[test]
fn pending_records_decide_the_later_submits_of_their_group() {
    let k1 = |expected| record("s", Some("k1"), "one", expected);
    let k2 = |expected| record("s", Some("k2"), "two", expected);
    let k3 = |expected| record("s", Some("k3"), "three", expected);
    let k9 = || record("t", Some("k9"), "new", Empty);
    let k10 = |expected| record("t", Some("k10"), "next", expected);
    let retry = |positions| Outcome::AfterFlush(done(positions, true));
    let durable = |positions| Outcome::AtClassification(done(positions, true));

    let mut script = Script::new();
    script.group(vec![
        sub([k1(Empty), k2(At(0))]),
        sub([k3(At(1))]),
        sub([k1(Empty), k2(At(0))]),
        sub([k2(Empty)]),
        sub([k3(At(1)), k1(Empty)]),
        sub([k9(), record("s", Some("k2"), "changed", At(7))]),
        sub([k1(Empty), record("s", None, "tail", At(2))]),
        sub([record("s", None, "late", At(0))]),
        sub([k9()]),
    ]);
    script.group(vec![
        sub([k3(Empty), k1(Empty)]),
        sub([k10(At(0))]),
        sub([k10(At(0)), k2(Empty)]),
    ]);
    script.open();
    script.group(vec![
        sub([k10(Empty), k1(Empty)]),
        sub([record("t", Some("k9"), "other", At(1))]),
    ]);

    let model = predict(&script);
    assert_eq!(
        model.steps[1].outcomes,
        [
            appended(&[(1, 0), (2, 1)]),
            appended(&[(3, 2)]),
            retry(&[(1, 0), (2, 1)]),
            retry(&[(2, 1)]),
            retry(&[(3, 2), (1, 0)]),
            rejected(Reject::KeyConflict { index: 1 }),
            rejected(Reject::PartialRetry),
            rejected(Reject::WrongRevision {
                index: 0,
                actual: Some(2)
            }),
            appended(&[(4, 0)]),
        ]
    );
    assert_eq!(model.steps[1].held.watermark, 1);
    assert_eq!(model.steps[1].held.heads[&bytes("s")], None);
    assert_eq!(
        model.steps[2].outcomes,
        [
            durable(&[(3, 2), (1, 0)]),
            appended(&[(6, 1)]),
            retry(&[(6, 1), (2, 1)])
        ]
    );
    assert_eq!(
        model.steps[4].outcomes,
        [
            durable(&[(6, 1), (1, 0)]),
            rejected(Reject::KeyConflict { index: 0 })
        ]
    );
    assert_eq!(model.end.watermark, 10);

    let log = drive(&script, &options());
    equal(["the model", "the log"], &model, &log).unwrap_or_else(|message| panic!("{message}"));
}

/// Given a group with a wrong revision after a good record on the same stream, a partial retry and a key
/// conflict, each with a fresh key, and a repeated key, between valid appends
/// When later appends use that stream and those fresh keys, and the group runs again without the four rejections
/// Then each valid append gets the same offset and revision in both runs
///
/// Cases: REV-NO-OFFSET
#[test]
fn rejected_submits_leave_the_next_offsets_unchanged() {
    let mut script = Script::new();
    script.group(vec![
        sub([
            record("s", Some("k5"), "a", Empty),
            record("s", None, "b", At(3)),
        ]),
        sub([record("s", Some("k1"), "c", Empty)]),
        sub([
            record("s", Some("k1"), "c", Empty),
            record("s", Some("k3"), "d", At(0)),
        ]),
        sub([
            record("s", Some("k4"), "e", At(0)),
            record("s", Some("k1"), "x", At(0)),
        ]),
        sub([
            record("s", Some("k2"), "f", At(0)),
            record("s", Some("k2"), "f", At(1)),
        ]),
        sub([record("s", Some("k2"), "f", At(0))]),
        sub([
            record("s", Some("k3"), "d", At(1)),
            record("s", Some("k4"), "e", At(2)),
            record("s", Some("k5"), "a", At(3)),
        ]),
    ]);

    let full = drive(&script, &options());
    assert_eq!(
        full.steps[1].outcomes,
        [
            rejected(Reject::WrongRevision {
                index: 1,
                actual: Some(0)
            }),
            appended(&[(1, 0)]),
            rejected(Reject::PartialRetry),
            rejected(Reject::KeyConflict { index: 1 }),
            Outcome::Refused,
            appended(&[(2, 1)]),
            appended(&[(3, 2), (4, 3), (5, 4)]),
        ]
    );
    let erased = full.rejected();
    assert_eq!(erased, [(1, 0), (1, 2), (1, 3), (1, 4)]);
    let kept = drive(&script.without(&erased), &options());
    assert_eq!(
        kept.steps[1].outcomes,
        [
            appended(&[(1, 0)]),
            appended(&[(2, 1)]),
            appended(&[(3, 2), (4, 3), (5, 4)])
        ]
    );
    equal(
        [
            "the full run less the rejected answers",
            "the run without them",
        ],
        &full.without(&erased),
        &kept,
    )
    .unwrap_or_else(|message| panic!("{message}"));
}
