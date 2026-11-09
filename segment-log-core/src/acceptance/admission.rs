use super::support::{bounded, fixtures::*};
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::{path::Path, sync::Arc};

/// Given invalid structure and a valid request larger than the configured group
/// When append is called
/// Then it refuses before writing and the next valid request still gets offset zero
/// Cases: ADM-PRECEDENCE, ADM-SIZE, KEY-REPEATED
#[test]
fn malformed_and_oversized_requests_are_refused_before_admission() {
    bounded(|| {
        let sim = Sim::new(8);
        let log = open(&sim);
        assert!(matches!(
            log.append(Submit { records: vec![] }),
            Err(AppendError::Invalid(_))
        ));
        let mut bad = submit(b"");
        assert!(matches!(log.append(bad), Err(AppendError::Invalid(_))));
        bad = submit(b"s");
        bad.records[0].key = Some(vec![]);
        assert!(matches!(log.append(bad), Err(AppendError::Invalid(_))));
        bad = submit(b"s");
        bad.records.push(submit(b"t").records.remove(0));
        assert!(matches!(log.append(bad), Err(AppendError::Invalid(_))));
        bad = submit(b"s");
        bad.records[0].payload = vec![0; 1024 * 1024];
        assert!(matches!(log.append(bad), Err(AppendError::TooLarge)));
        append(&log, b"s").wait().unwrap();
        assert_record(&log);
    });
}

/// Given a held group and a byte budget sufficient for one request only
/// When another eligible request arrives or a single request exceeds the whole budget
/// Then Full and TooLarge are immediate, and answering releases the first reservation
/// Cases: ADM-BUDGETS, ADM-RESERVATION, ADM-CHARGE
#[test]
fn pending_bytes_remain_charged_until_the_answer() {
    bounded(|| {
        let sim = Sim::new(10);
        let log = SegmentLog::with_storage(
            Path::new("/log"),
            Options {
                pending_byte_budget: 4096,
                ..options()
            },
            Arc::new(sim.clone()),
        )
        .unwrap();
        let make = || {
            let mut s = submit(b"s");
            s.records[0].payload = Vec::with_capacity(2500);
            s
        };
        let hold = sim.hold(Call::new(SEG, Purpose::Group, Op::SyncData));
        let pending = log.append(make()).unwrap();
        hold.wait();
        assert!(matches!(log.append(make()), Err(AppendError::Full)));
        let mut huge = make();
        huge.records[0].payload = Vec::with_capacity(5000);
        assert!(matches!(log.append(huge), Err(AppendError::TooLarge)));
        hold.release(None);
        pending.wait().unwrap();
        assert!(log.append(make()).is_ok());
    });
}
