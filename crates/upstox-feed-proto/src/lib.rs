//! Wire-format decoder for Upstox Market Data Feed V3.
//!
//! This crate only decodes raw bytes into the generated schema types and
//! exposes a small diagnostic projection. It does not normalize prices or
//! timestamps, map instruments, or produce trading events. The raw frame
//! remains authoritative.

pub mod decode;
pub mod proto {
    include!(concat!(
        env!("OUT_DIR"),
        "/com.upstox.marketdatafeederv3udapi.rpc.proto.rs"
    ));
}

pub use decode::{
    DecodeError, FeedKind, FeedResponseDiagnostic, FeedResponseType, FeedSummary,
    decode_feed_response, decode_feed_response_diagnostic, response_type_from_raw,
};
