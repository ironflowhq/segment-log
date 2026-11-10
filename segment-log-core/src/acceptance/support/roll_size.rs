/// The roll size of the test fixtures. A small `SEGMENT_LOG_TEST_ROLL_BYTES` reruns every scenario
/// built on them across rolls. Integration tests include this file by path.
pub(crate) fn roll_bytes(default: u64) -> u64 {
    match std::env::var("SEGMENT_LOG_TEST_ROLL_BYTES") {
        Err(std::env::VarError::NotPresent) => default,
        Ok(value) => value
            .parse()
            .expect("SEGMENT_LOG_TEST_ROLL_BYTES is a number of bytes"),
        Err(error) => panic!("SEGMENT_LOG_TEST_ROLL_BYTES: {error}"),
    }
}
