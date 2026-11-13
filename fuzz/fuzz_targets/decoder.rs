#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| segment_log_core::fuzzing::decode_frame(data));
