use std::panic::{catch_unwind, AssertUnwindSafe};

use android_use::{batch, protocol, selector, MAX_PROTOCOL_FRAME};

fn corpus(seed: &mut u64, len: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        *seed ^= *seed << 7;
        *seed ^= *seed >> 9;
        *seed ^= *seed << 8;
        bytes.push(*seed as u8);
    }
    bytes
}

fn framed(payload: &[u8]) -> Vec<u8> {
    let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(payload);
    frame
}

#[test]
fn protocol_decoders_are_panic_free_and_bounded_on_deterministic_corpus() {
    let mut seed = 0xA_u64;
    let lengths = [0, 1, 2, 3, 4, 7, 15, 31, 63, 127, 255, 511, 1024, 4096];
    for length in lengths {
        let payload = corpus(&mut seed, length);
        let framed_payload = framed(&payload);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = protocol::read_daemon_request(&mut framed_payload.as_slice());
            let _ = protocol::read_native_response(&mut framed_payload.as_slice());
        }));
        assert!(
            result.is_ok(),
            "decoder panicked for payload length {length}"
        );
    }

    let oversized = ((MAX_PROTOCOL_FRAME as u32) + 1).to_le_bytes();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _ = protocol::read_daemon_request(&mut oversized.as_slice());
    }));
    assert!(result.is_ok(), "oversized frame decoder panicked");
}

#[test]
fn parser_corpus_is_panic_free_for_agent_control_text() {
    let mut seed = 0xC0DEC0DE_u64;
    for length in [0, 1, 2, 7, 16, 64, 256, 1024, 4096] {
        let bytes = corpus(&mut seed, length);
        let text = String::from_utf8_lossy(&bytes);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = batch::parse(&text);
            let _ = selector::Selector::parse(&text);
        }));
        assert!(result.is_ok(), "parser panicked for text length {length}");
    }
}
