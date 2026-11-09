use super::support::{bounded, fixtures::*};
use crate::storage::{Op, Purpose};
use crate::*;
use std::path::Path;

/// Given an empty log and one successfully committed stream
/// When a new submit expects a nonexistent revision or Empty on the existing stream
/// Then it is rejected without consuming an offset and a correct At append gets revision 1
/// Cases: REV-EXPECTED, REV-NO-OFFSET
#[test]
fn wrong_revision_does_not_consume_a_position() {
    bounded(|| {
        let sim = Sim::new(7);
        let log = open(&sim);
        let mut wrong = submit(b"s");
        wrong.records[0].expected = ExpectedRevision::At(0);
        assert!(matches!(
            log.append(wrong).unwrap().wait(),
            Err(CommitError::WrongRevision {
                index: 0,
                actual: None
            })
        ));
        append(&log, b"s").wait().unwrap();
        let mut wrong = submit(b"s");
        wrong.records[0].key = None;
        assert!(matches!(
            log.append(wrong).unwrap().wait(),
            Err(CommitError::WrongRevision {
                index: 0,
                actual: Some(0)
            })
        ));
        let mut next = submit(b"s");
        next.records[0].key = None;
        next.records[0].expected = ExpectedRevision::At(0);
        let result = log.append(next).unwrap().wait().unwrap();
        assert_eq!(
            (result.records[0].offset, result.records[0].revision),
            (1, 1)
        );
        assert_eq!(sim.count(Path::new(SEG), Purpose::Group, &Op::SyncData), 2);
    });
}
