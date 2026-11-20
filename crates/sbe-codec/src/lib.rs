#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Self-contained decoder for the challenge SBE order-book schema.
//!
//! This is the active home for the protocol obligation retained under
//! `.local/extras/cpp`. It implements schema id 1, version 2, templates 1 and 2, with
//! no dependency on the removed predecessor monolith types and no use of the
//! numeric/book oracle as SBE evidence.

mod decode;

pub use decode::{
    DecodeResult, //
    ENTRY_BYTES,
    GROUP_HEADER_BYTES,
    HEADER_BYTES,
    MIN_CHUNK_BYTES,
    SCHEMA_ID,
    SCHEMA_VERSION,
    SNAPSHOT_BLOCK_BYTES,
    SNAPSHOT_TEMPLATE,
    SbeError,
    SbeLevel,
    SbeMessage,
    UPDATE_BLOCK_BYTES,
    UPDATE_TEMPLATE,
    decode_capture,
    decode_message,
    encode_message,
    format_dump_line,
};
