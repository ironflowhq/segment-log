use super::*;

/// A reader may lose a read only to a crash, and the trace holds every read that a crash canceled.
fn assert_only_a_crash_canceled_reads(sim: &Sim, canceled: usize) {
    let overtaken = sim
        .trace()
        .1
        .iter()
        .filter(|event| {
            matches!(event, Event::Canceled(call)
                if call.purpose == Purpose::Lookup && matches!(call.op, Op::Read(..)))
        })
        .count();
    assert!(
        canceled <= overtaken,
        "the reader lost {canceled} reads, but a crash canceled {overtaken}"
    );
}

pub(super) fn workload(sim: &Sim, shared: &Arc<Mutex<Ledger>>, variant: usize) {
    let seed = shared.lock().unwrap().seed;
    let mut rng = Rng(seed);
    let anchor = literal(0, 1, 1 + rng.pick(30), 0);
    let boot = literal(1, 1, 1 + rng.pick(30), 0);
    let mut originals: Vec<_> = (0..3 + rng.pick(3))
        .map(|lane| {
            literal(
                10 + lane as u64,
                2 + rng.pick(3),
                [1, 19, 513, 4097][rng.pick(4)],
                if lane == 0 { 0 } else { rng.next() % 3 },
            )
        })
        .collect();
    rng.shuffle(&mut originals);
    let mut following = literal(70, 1, 1 + rng.pick(50), 0);
    following[0].stream = originals[0][0].stream.clone();
    following[0].expected = Some(originals[0].len() as u64 - 1);
    originals.push(following);
    let all: Vec<_> = originals.iter().flatten().cloned().collect();
    shared.lock().unwrap().options.group_byte_cap = body(&all);
    let log = Arc::new(open(sim, &shared.lock().unwrap().options));
    let mut predicted = vec![];
    let first = Caller::start(&log, shared, anchor.clone(), true);
    first.finish();
    oracle::predict(&anchor, &mut predicted);
    let boot_position = crate::acceptance::support::raw::bytes(sim).len();
    let held = sim.hold(flush());
    let bootstrap = Caller::start(&log, shared, boot.clone(), true);
    held.wait();
    oracle::predict(&boot, &mut predicted);
    let boot_len = crate::acceptance::support::raw::bytes(sim).len() - boot_position;
    let mut callers = vec![];
    let mut planned = vec![];
    for inputs in &originals {
        let expected = oracle::predict(inputs, &mut predicted);
        let caller = Caller::start(&log, shared, inputs.clone(), true);
        planned.push((caller.id, expected));
        callers.push(caller);
    }
    let keyed = originals
        .iter()
        .find(|q| q.len() >= 2 && q.iter().all(|r| r.key.is_some()))
        .unwrap();
    let mut trials = vec![
        keyed.clone(),
        vec![keyed[0].clone()],
        keyed.iter().rev().cloned().collect(),
        vec![keyed[1].clone(), anchor[0].clone()],
        anchor.clone(),
    ];
    let mut conflict = keyed[0].clone();
    conflict.payload.push(0xff);
    trials.push(vec![literal(80, 1, 7, 0)[0].clone(), conflict]);
    trials.push(vec![keyed[0].clone(), literal(81, 1, 7, 1)[0].clone()]);
    trials.push(vec![
        literal(82, 1, 7, 0)[0].clone(),
        Input {
            key: None,
            ..keyed[0].clone()
        },
    ]);
    trials.push(vec![keyed[0].clone(), keyed[0].clone()]);
    for q in &mut trials[..4] {
        for r in q {
            r.expected = Some(rng.next());
        }
    }
    rng.shuffle(&mut trials);
    for inputs in trials {
        let expected = oracle::predict(&inputs, &mut predicted);
        let caller = Caller::start(&log, shared, inputs, false);
        planned.push((caller.id, expected));
        callers.push(caller);
    }
    let mut stale = literal(83, 2, 5, 0);
    stale[1].stream = anchor[0].stream.clone();
    stale[1].expected = None;
    Caller::start(&log, shared, stale, false).abandon();
    let mut too_large = literal(90, 1, 1, 0);
    too_large[0].spare = shared.lock().unwrap().options.pending_byte_budget + 1;
    let oversized = Caller::start(&log, shared, too_large, false);
    let oversized_id = oversized.id;
    oversized.finish();
    assert_eq!(
        shared.lock().unwrap().requests[oversized_id].answer,
        Answer::Refused("TooLarge".into())
    );
    let lookup_failure = variant == 7;
    if lookup_failure {
        let failing = Caller::start(&log, shared, boot.clone(), false);
        planned.push((failing.id, Answer::Unknown));
        callers.push(failing);
    }
    let deferred = Caller::start(&log, shared, literal(91, 2, 23, 0), true);
    let deferred_id = deferred.id;
    let next = sim.hold(if lookup_failure {
        Call::new(
            SEG,
            Purpose::Lookup,
            Op::Read(boot_position as u64, boot_len),
        )
    } else if matches!(variant, 1 | 4 | 5) {
        Call::new(
            ANY_SEG,
            Purpose::Group,
            Op::Append(frame(2, &all, &predicted[..2])),
        )
    } else {
        flush()
    });
    held.release(None);
    bootstrap.finish();
    next.wait();
    let queued = Caller::start(&log, shared, keyed.clone(), false);
    let queued_id = queued.id;
    let mode = shared.lock().unwrap().mode;
    observe(&log, shared, 0, 2 + rng.pick(4));
    observe(&log, shared, 2, 7);
    observe(&log, shared, u64::MAX, 0);
    for (c, (id, expected)) in callers.iter().zip(&planned) {
        assert_eq!(c.id, *id);
        let waiting = c.command(Command::Probe);
        let actual = shared.lock().unwrap().requests[c.id].answer.clone();
        match expected {
            Answer::Ack(false, _) | Answer::Unknown => assert!(waiting),
            Answer::Ack(true, positions) if positions.iter().any(|(o, _)| *o >= 2) => {
                assert!(waiting)
            }
            _ => {
                assert!(!waiting);
                assert_eq!(&actual, expected);
            }
        }
    }
    let abandoned = callers.remove(0);
    let abandoned_id = abandoned.id;
    abandoned.abandon();
    let before_fault = sim.trace().1.len();
    let reader = Reader::start(&log, shared);
    match variant {
        0 => next.release(None),
        1..=3 => {
            sim.crash(mode, seed);
            next.release(None);
        }
        4 => next.release(Some(Fault::Partial(
            1 + rng.pick(frame(2, &all, &predicted[..2]).len()),
        ))),
        5 => next.release(Some(Fault::NoSpace)),
        6 => next.release(Some(Fault::error(std::io::ErrorKind::Other).unwrap())),
        7 => next.release(Some(match rng.pick(3) {
            0 => Fault::Eof,
            1 => Fault::Corrupt(8, 1),
            _ => Fault::NoSpace,
        })),
        _ => unreachable!(),
    }
    for caller in callers {
        caller.finish();
    }
    deferred.finish();
    queued.finish();
    assert_only_a_crash_canceled_reads(sim, reader.finish());
    if variant == 0 {
        observe(&log, shared, 1, 100);
    }
    let watermark = log.watermark();
    drop(log);
    let l = shared.lock().unwrap();
    for (id, expected) in planned {
        if id == abandoned_id {
            assert_eq!(l.requests[id].answer, Answer::Unknown);
            continue;
        }
        let expected = match expected {
            Answer::Ack(false, _) if variant != 0 => Answer::Unknown,
            Answer::Ack(true, ref ps) if variant != 0 && ps.iter().any(|(o, _)| *o >= 2) => {
                Answer::Unknown
            }
            a => a,
        };
        assert_eq!(
            l.requests[id].answer, expected,
            "variant={variant} request={id}"
        );
    }
    if variant != 0 {
        for id in [deferred_id, queued_id] {
            assert_eq!(l.requests[id].answer, Answer::Refused("Poisoned".into()));
        }
        assert_eq!(watermark, 2);
        let events = sim.trace().1;
        let failed = events[before_fault..]
            .iter()
            .position(|e| matches!(e, Event::Call(_, Some(_), _) | Event::Canceled(_)))
            .unwrap()
            + before_fault;
        assert!(!events[failed+1..].iter().any(|e| matches!(e,Event::Call(c,_,_) if c.purpose==Purpose::Group && matches!(c.op,Op::Append(_)|Op::SyncData))),"no I/O after poison");
    }
    drop(l);
    if variant == 0 || (variant >= 4 && seed.is_multiple_of(2)) {
        sim.crash(mode, seed);
    }
}

pub(super) fn pressure(sim: &Sim, shared: &Arc<Mutex<Ledger>>) {
    let seed = shared.lock().unwrap().seed;
    let mut rng = Rng(seed);
    {
        let mut l = shared.lock().unwrap();
        l.options.queue_capacity = 1 + rng.pick(3);
        l.options.pending_byte_budget = 4096 + rng.pick(1024);
        l.options.group_byte_cap = 128 + rng.pick(64);
    }
    let log = Arc::new(open(sim, &shared.lock().unwrap().options));
    let held = sim.hold(flush());
    let original = Caller::start(&log, shared, literal(0, 2, 7, 0), true);
    held.wait();
    let mut callers = vec![];
    for i in 1..12 {
        let mut inputs = literal(i, 2, 1 + rng.pick(15), rng.next() % 3);
        if i % 3 == 0 {
            inputs[0].spare = 2000;
        }
        if i == 11 {
            inputs[0].spare = 6000;
        }
        callers.push(Caller::prepare(&log, shared, inputs, true));
    }
    for c in &mut callers {
        c.release();
    }
    for c in &callers {
        c.ready();
    }
    observe(&log, shared, 0, 10);
    for c in &callers {
        c.command(Command::Probe);
    }
    {
        let l = shared.lock().unwrap();
        let admitted: Vec<_> = l
            .requests
            .iter()
            .filter(|q| q.answer == Answer::Unknown)
            .collect();
        let remaining =
            l.options.pending_byte_budget - oracle::charge(&l.requests[0].inputs).unwrap();
        assert!(
            l.requests[1..]
                .iter()
                .any(|q| oracle::charge(&q.inputs).unwrap() <= remaining)
        );
        assert!(
            admitted.len() > 1,
            "eligible competitors must enter the empty queue"
        );
        assert!(admitted.len() - 1 <= l.options.queue_capacity);
        assert!(
            admitted
                .iter()
                .map(|q| oracle::charge(&q.inputs).unwrap())
                .sum::<usize>()
                <= l.options.pending_byte_budget
        );
        assert!(
            l.requests
                .iter()
                .any(|q| q.answer == Answer::Refused("Full".into()))
        );
        assert!(
            l.requests
                .iter()
                .any(|q| q.answer == Answer::Refused("TooLarge".into()))
        );
    }
    let reader = Reader::start(&log, shared);
    held.release(None);
    original.finish();
    for c in callers {
        c.finish();
    }
    assert_only_a_crash_canceled_reads(sim, reader.finish());
    observe(&log, shared, 0, 100);
    drop(log);
    let l = shared.lock().unwrap();
    sim.crash(l.mode, l.seed);
}
