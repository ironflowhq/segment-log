#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| segment_log_core::fuzzing::build_and_recover_log(data));
