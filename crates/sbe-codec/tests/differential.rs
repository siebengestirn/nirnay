#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Independent differential tests for the active SBE decoder.
//!
//! The oracle is the canonical dump produced by the C++ SBE decoder retained
//! under `extras/cpp`. It is a protocol oracle only; the numeric/book oracle is
//! a separate obligation and is not used here.

use sha2::{
    Digest, //
    Sha256,
};

use sbe_codec::{
    MIN_CHUNK_BYTES, //
    decode_capture,
    decode_message,
    encode_message,
    format_dump_line,
};

const GOLDEN: &[u8] = include_bytes!("golden/sample_replay.sbe");
const GOLDEN_SHA256: &str = "af291b403553b6e0d182f8001643e03a055e4d7e8b0dbbdfee54c3ad56f50e14";
const CPP_DUMP: &str = include_str!("golden/cpp_canonical_dump.txt");
const CPP_DUMP_SHA256: &str = "cba8bae7984408893e34a734b3db5e8598446932181504ffd04b7dca1ae9eaed";

fn rust_dump() -> String {
    let messages = decode_capture(GOLDEN, 4096).expect("golden decodes");
    let mut dump = String::new();
    for message in messages {
        dump.push_str(&format_dump_line(&message).expect("canonical dump line"));
        dump.push('\n');
    }
    dump
}

#[test]
fn golden_capture_matches_frozen_provenance() {
    assert_eq!(GOLDEN.len(), 4096, "the golden capture is 4096 bytes");
    assert_eq!(
        format!("{:x}", Sha256::digest(GOLDEN)),
        GOLDEN_SHA256,
        "the golden bytes match the frozen provenance digest"
    );
    assert_eq!(
        MIN_CHUNK_BYTES, 1024,
        "the retained harness minimum chunk size is preserved"
    );
}

#[test]
fn rust_dump_matches_the_independent_cpp_sbe_dump() {
    let dump = rust_dump();
    assert_eq!(
        format!("{:x}", Sha256::digest(CPP_DUMP.as_bytes())),
        CPP_DUMP_SHA256,
        "the captured C++ oracle dump is intact"
    );
    assert_eq!(
        dump.lines().count(),
        10,
        "all ten golden messages are dumped"
    );
    assert_eq!(
        dump, CPP_DUMP,
        "Rust and independent C++ SBE dumps are identical"
    );
}

#[test]
fn canonical_reencoding_is_byte_identical_to_the_golden() {
    let mut offset = 0;
    let mut messages = 0;
    while offset < GOLDEN.len() {
        if GOLDEN[offset..].iter().all(|byte| *byte == 0) {
            break;
        }
        let decoded = decode_message(GOLDEN, offset).expect("decode at offset");
        let encoded = encode_message(&decoded.message).expect("canonical encode");
        let again = decode_message(&encoded, 0).expect("decode re-encoded bytes");
        assert_eq!(
            again.message, decoded.message,
            "message survives an encode/decode round-trip"
        );
        assert_eq!(
            again.bytes_consumed,
            encoded.len(),
            "re-encoded message consumes exactly its own length"
        );
        assert_eq!(
            encoded.as_slice(),
            &GOLDEN[offset..offset + decoded.bytes_consumed],
            "canonical re-encoding is byte-identical to the original message"
        );
        offset += decoded.bytes_consumed;
        messages += 1;
    }
    assert_eq!(messages, 10, "the golden contains ten contiguous messages");
    assert_eq!(offset, 732, "messages end before zero padding");
}

#[test]
fn seven_malformed_cases_fail_closed() {
    let mut truncated = GOLDEN.to_vec();
    truncated.resize(10, 0);
    assert!(
        decode_capture(&truncated, 4096).is_err(),
        "a truncated header is refused"
    );

    let mut bad_template = GOLDEN.to_vec();
    bad_template[2] = 99;
    assert!(
        decode_capture(&bad_template, 4096).is_err(),
        "an unknown template is refused"
    );

    let mut bad_side = GOLDEN.to_vec();
    bad_side[162 + 8 + 32] = 7;
    assert!(
        decode_capture(&bad_side, 4096).is_err(),
        "an invalid SideEncoding is refused"
    );

    let mut bad_schema = GOLDEN.to_vec();
    bad_schema[4] = 99;
    assert!(
        decode_capture(&bad_schema, 4096).is_err(),
        "an unknown schema id is refused"
    );

    let mut bad_version = GOLDEN.to_vec();
    bad_version[6] = 99;
    assert!(
        decode_capture(&bad_version, 4096).is_err(),
        "an unknown schema version is refused"
    );

    let mut bad_group = GOLDEN.to_vec();
    bad_group[24 + 1] = 0x7f;
    assert!(
        decode_capture(&bad_group, 4096).is_err(),
        "an invalid group block length is refused"
    );

    assert!(
        decode_capture(GOLDEN, 512).is_err(),
        "a chunk below the minimum is refused"
    );

    let mut torn = vec![0_u8; 4096 + 200];
    let one = encode_message(&decode_message(GOLDEN, 0).expect("first message").message)
        .expect("encoded first message");
    let start = 4096 - 10;
    torn[start..start + one.len()].copy_from_slice(&one);
    assert!(
        decode_capture(&torn, 4096).is_err(),
        "a message crossing an aligned chunk boundary is refused"
    );
}
