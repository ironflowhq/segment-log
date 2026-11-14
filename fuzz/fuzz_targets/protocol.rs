//! `FUZZ-PROTOCOL` (testing.md): the server's header and request decoder on arbitrary bytes. A
//! length above the cap is refused before any body is read, and a request that decodes encodes back
//! to the same bytes.
//!
//! Cases: FUZZ-PROTOCOL
#![no_main]

use segment_log_server::protocol::{self, Admit, HEADER_LEN};

// The cap of the smallest group, which is the 64 KiB floor; -max_len reaches past it.
const FRAME_CAP: usize = 64 * 1024;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((header, body)) = data.split_first_chunk::<HEADER_LEN>() else {
        return;
    };
    let Ok(header) = protocol::decode_header(*header) else {
        return;
    };
    match protocol::admit(header, FRAME_CAP) {
        Ok(Admit::Read) => assert!(header.body_len <= FRAME_CAP),
        Ok(Admit::Discard) => {
            assert!(header.body_len > FRAME_CAP && header.kind == protocol::APPEND);
            return;
        }
        Err(_) => return,
    }
    let Some(body) = body.get(..header.body_len) else {
        return;
    };
    if let Ok(request) = protocol::decode_request(header.kind, body) {
        assert_eq!(protocol::encode_request(&request), data[..HEADER_LEN + body.len()]);
    }
});
