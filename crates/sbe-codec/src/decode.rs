//! Challenge SBE wire decoder, encoder and canonical dump formatter.
//!
//! All multi-byte integers are little-endian. Every structural read is bounded
//! and fails closed; no read falls back to a default value.

use std::fmt;

use thiserror::Error;

/// Schema id declared by the frozen challenge schema.
pub const SCHEMA_ID: u16 = 1;
/// Schema version declared by the frozen challenge schema.
pub const SCHEMA_VERSION: u16 = 2;
/// Full order-book snapshot template id.
pub const SNAPSHOT_TEMPLATE: u16 = 1;
/// Single price-level update template id.
pub const UPDATE_TEMPLATE: u16 = 2;

/// Fixed SBE message header size.
pub const HEADER_BYTES: usize = 8;
/// Fixed snapshot body block size.
pub const SNAPSHOT_BLOCK_BYTES: usize = 16;
/// Fixed update body block size.
pub const UPDATE_BLOCK_BYTES: usize = 33;
/// Repeating-group dimension header size.
pub const GROUP_HEADER_BYTES: usize = 4;
/// One price/quantity group entry size.
pub const ENTRY_BYTES: usize = 16;
/// Smallest aligned capture chunk the retained harness accepts.
pub const MIN_CHUNK_BYTES: usize = 1024;

/// Fail-closed decode and encode errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SbeError {
    #[error("truncated {0}")]
    Truncated(&'static str),
    #[error("unknown schema id {0}")]
    UnknownSchemaId(u16),
    #[error("unknown schema version {0}")]
    UnknownSchemaVersion(u16),
    #[error("unknown template id {0}")]
    UnknownTemplate(u16),
    #[error("block length mismatch for template {template}: expected {expected} got {actual}")]
    BlockLengthMismatch {
        template: u16,
        expected: usize,
        actual: usize,
    },
    #[error("invalid group entry block length {0}")]
    InvalidGroupEntryLength(u16),
    #[error("invalid side encoding {0}")]
    InvalidSide(u8),
    #[error("chunk size {0} is below the minimum")]
    ChunkTooSmall(usize),
    #[error("message tears across an aligned chunk boundary")]
    MessageTear,
    #[error("symbol is not valid UTF-8")]
    InvalidSymbol,
    #[error("symbol length exceeds the uint32 field")]
    SymbolTooLong,
    #[error("group size exceeds the uint16 field")]
    GroupTooLarge,
    #[error("update message is missing {0}")]
    MissingUpdateField(&'static str),
}

/// One decoded price level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SbeLevel {
    pub price: i64,
    pub quantity: i64,
}

impl SbeLevel {
    fn new(price: i64, quantity: i64) -> Self {
        Self { price, quantity }
    }
}

/// Canonical message representation shared with the C++ decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SbeMessage {
    pub template_id: u16,
    pub received_at_us: u64,
    pub exchange_timestamp_us: u64,
    pub symbol: String,
    pub bids: Vec<SbeLevel>,
    pub asks: Vec<SbeLevel>,
    pub price_level: Option<i64>,
    pub quantity: Option<i64>,
    pub side: Option<u8>,
}

impl SbeMessage {
    fn new(template_id: u16) -> Self {
        Self {
            template_id,
            received_at_us: 0,
            exchange_timestamp_us: 0,
            symbol: String::new(),
            bids: Vec::new(),
            asks: Vec::new(),
            price_level: None,
            quantity: None,
            side: None,
        }
    }
}

/// A decoded message plus the exact number of bytes it consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeResult {
    pub message: SbeMessage,
    pub bytes_consumed: usize,
}

fn need(bytes: &[u8], offset: usize, length: usize, what: &'static str) -> Result<(), SbeError> {
    if offset > bytes.len() || length > bytes.len() - offset {
        return Err(SbeError::Truncated(what));
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, SbeError> {
    need(bytes, offset, 2, "u16")?;
    let slice: [u8; 2] = bytes[offset..offset + 2]
        .try_into()
        .map_err(|_| SbeError::Truncated("u16"))?;
    Ok(u16::from_le_bytes(slice))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, SbeError> {
    need(bytes, offset, 4, "u32")?;
    let slice: [u8; 4] = bytes[offset..offset + 4]
        .try_into()
        .map_err(|_| SbeError::Truncated("u32"))?;
    Ok(u32::from_le_bytes(slice))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, SbeError> {
    need(bytes, offset, 8, "u64")?;
    let slice: [u8; 8] = bytes[offset..offset + 8]
        .try_into()
        .map_err(|_| SbeError::Truncated("u64"))?;
    Ok(u64::from_le_bytes(slice))
}

fn read_group(bytes: &[u8], pos: &mut usize) -> Result<Vec<SbeLevel>, SbeError> {
    need(bytes, *pos, GROUP_HEADER_BYTES, "group header")?;
    let entry_block = read_u16(bytes, *pos)?;
    let count = read_u16(bytes, *pos + 2)?;
    if entry_block as usize != ENTRY_BYTES {
        return Err(SbeError::InvalidGroupEntryLength(entry_block));
    }
    *pos += GROUP_HEADER_BYTES;
    let mut levels = Vec::with_capacity(count as usize);
    for _ in 0..count {
        need(bytes, *pos, ENTRY_BYTES, "group entry")?;
        let price = read_u64(bytes, *pos)? as i64;
        let quantity = read_u64(bytes, *pos + 8)? as i64;
        levels.push(SbeLevel::new(price, quantity));
        *pos += ENTRY_BYTES;
    }
    Ok(levels)
}

fn read_string(bytes: &[u8], pos: &mut usize) -> Result<String, SbeError> {
    need(bytes, *pos, 4, "var-data length")?;
    let length = read_u32(bytes, *pos)? as usize;
    *pos += 4;
    need(bytes, *pos, length, "var-data")?;
    let text =
        std::str::from_utf8(&bytes[*pos..*pos + length]).map_err(|_| SbeError::InvalidSymbol)?;
    *pos += length;
    Ok(text.to_owned())
}

fn append_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn append_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn append_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn append_group(out: &mut Vec<u8>, levels: &[SbeLevel]) -> Result<(), SbeError> {
    if levels.len() > u16::MAX as usize {
        return Err(SbeError::GroupTooLarge);
    }
    append_u16(out, ENTRY_BYTES as u16);
    append_u16(out, levels.len() as u16);
    for level in levels {
        append_u64(out, level.price as u64);
        append_u64(out, level.quantity as u64);
    }
    Ok(())
}

fn append_symbol(out: &mut Vec<u8>, symbol: &str) -> Result<(), SbeError> {
    let length = u32::try_from(symbol.len()).map_err(|_| SbeError::SymbolTooLong)?;
    append_u32(out, length);
    out.extend_from_slice(symbol.as_bytes());
    Ok(())
}

fn check_block(template_id: u16, expected: usize, actual: usize) -> Result<(), SbeError> {
    if actual == expected {
        Ok(())
    } else {
        Err(SbeError::BlockLengthMismatch {
            template: template_id,
            expected,
            actual,
        })
    }
}

/// Decode one message at `offset`.
pub fn decode_message(buffer: &[u8], offset: usize) -> Result<DecodeResult, SbeError> {
    need(buffer, offset, HEADER_BYTES, "message header")?;
    let block_length = read_u16(buffer, offset)? as usize;
    let template_id = read_u16(buffer, offset + 2)?;
    let schema_id = read_u16(buffer, offset + 4)?;
    let schema_version = read_u16(buffer, offset + 6)?;
    if schema_id != SCHEMA_ID {
        return Err(SbeError::UnknownSchemaId(schema_id));
    }
    if schema_version != SCHEMA_VERSION {
        return Err(SbeError::UnknownSchemaVersion(schema_version));
    }
    let body = offset + HEADER_BYTES;
    let mut message = SbeMessage::new(template_id);

    if template_id == SNAPSHOT_TEMPLATE {
        check_block(template_id, SNAPSHOT_BLOCK_BYTES, block_length)?;
        need(buffer, body, block_length, "snapshot block")?;
        message.received_at_us = read_u64(buffer, body)?;
        message.exchange_timestamp_us = read_u64(buffer, body + 8)?;
        let mut pos = body + block_length;
        message.bids = read_group(buffer, &mut pos)?;
        message.asks = read_group(buffer, &mut pos)?;
        message.symbol = read_string(buffer, &mut pos)?;
        return Ok(DecodeResult {
            message,
            bytes_consumed: pos - offset,
        });
    }

    if template_id == UPDATE_TEMPLATE {
        check_block(template_id, UPDATE_BLOCK_BYTES, block_length)?;
        need(buffer, body, block_length, "update block")?;
        message.received_at_us = read_u64(buffer, body)?;
        message.exchange_timestamp_us = read_u64(buffer, body + 8)?;
        message.price_level = Some(read_u64(buffer, body + 16)? as i64);
        message.quantity = Some(read_u64(buffer, body + 24)? as i64);
        let side = buffer[body + 32];
        if side > 1 {
            return Err(SbeError::InvalidSide(side));
        }
        message.side = Some(side);
        let mut pos = body + block_length;
        message.symbol = read_string(buffer, &mut pos)?;
        return Ok(DecodeResult {
            message,
            bytes_consumed: pos - offset,
        });
    }

    Err(SbeError::UnknownTemplate(template_id))
}

/// Encode one message to its canonical bytes.
pub fn encode_message(message: &SbeMessage) -> Result<Vec<u8>, SbeError> {
    let mut out = Vec::new();
    match message.template_id {
        SNAPSHOT_TEMPLATE => {
            append_u16(&mut out, SNAPSHOT_BLOCK_BYTES as u16);
            append_u16(&mut out, SNAPSHOT_TEMPLATE);
            append_u16(&mut out, SCHEMA_ID);
            append_u16(&mut out, SCHEMA_VERSION);
            append_u64(&mut out, message.received_at_us);
            append_u64(&mut out, message.exchange_timestamp_us);
            append_group(&mut out, &message.bids)?;
            append_group(&mut out, &message.asks)?;
            append_symbol(&mut out, &message.symbol)?;
        }
        UPDATE_TEMPLATE => {
            let price = message
                .price_level
                .ok_or(SbeError::MissingUpdateField("price_level"))?;
            let quantity = message
                .quantity
                .ok_or(SbeError::MissingUpdateField("quantity"))?;
            let side = message.side.ok_or(SbeError::MissingUpdateField("side"))?;
            if side > 1 {
                return Err(SbeError::InvalidSide(side));
            }
            append_u16(&mut out, UPDATE_BLOCK_BYTES as u16);
            append_u16(&mut out, UPDATE_TEMPLATE);
            append_u16(&mut out, SCHEMA_ID);
            append_u16(&mut out, SCHEMA_VERSION);
            append_u64(&mut out, message.received_at_us);
            append_u64(&mut out, message.exchange_timestamp_us);
            append_u64(&mut out, price as u64);
            append_u64(&mut out, quantity as u64);
            out.push(side);
            append_symbol(&mut out, &message.symbol)?;
        }
        other => return Err(SbeError::UnknownTemplate(other)),
    }
    Ok(out)
}

/// Decode every message in an aligned capture.
pub fn decode_capture(bytes: &[u8], chunk_size: usize) -> Result<Vec<SbeMessage>, SbeError> {
    if chunk_size < MIN_CHUNK_BYTES {
        return Err(SbeError::ChunkTooSmall(chunk_size));
    }
    let mut messages = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let mut chunk_end = offset + (chunk_size - offset % chunk_size);
        if chunk_end > bytes.len() {
            chunk_end = bytes.len();
        }
        if offset + HEADER_BYTES > chunk_end {
            if offset < bytes.len()
                && offset + HEADER_BYTES <= bytes.len()
                && chunk_end == bytes.len()
            {
                break;
            }
            offset = chunk_end;
            continue;
        }
        let block_length = read_u16(bytes, offset)?;
        let template_id = read_u16(bytes, offset + 2)?;
        if template_id == 0 && block_length == 0 {
            offset = chunk_end;
            continue;
        }
        let result = decode_message(bytes, offset)?;
        if offset + result.bytes_consumed > chunk_end && chunk_end != bytes.len() {
            return Err(SbeError::MessageTear);
        }
        messages.push(result.message);
        offset += result.bytes_consumed;
        if offset >= chunk_end {
            offset = chunk_end;
        }
    }
    Ok(messages)
}

/// Format the canonical one-line dump shared with the C++ decoder.
pub fn format_dump_line(message: &SbeMessage) -> Result<String, SbeError> {
    match message.template_id {
        SNAPSHOT_TEMPLATE => {
            let bids = message
                .bids
                .iter()
                .map(|level| format!("{}:{}", level.price, level.quantity))
                .collect::<Vec<_>>()
                .join(",");
            let asks = message
                .asks
                .iter()
                .map(|level| format!("{}:{}", level.price, level.quantity))
                .collect::<Vec<_>>()
                .join(",");
            Ok(format!(
                "snapshot {}:{}:{}:bids={}:asks={}",
                message.symbol, message.received_at_us, message.exchange_timestamp_us, bids, asks
            ))
        }
        UPDATE_TEMPLATE => {
            let side = message.side.ok_or(SbeError::MissingUpdateField("side"))?;
            let price = message
                .price_level
                .ok_or(SbeError::MissingUpdateField("price_level"))?;
            let quantity = message
                .quantity
                .ok_or(SbeError::MissingUpdateField("quantity"))?;
            Ok(format!(
                "update {}:{}:{}:side={}:price={}:qty={}",
                message.symbol,
                message.received_at_us,
                message.exchange_timestamp_us,
                side,
                price,
                quantity
            ))
        }
        other => Err(SbeError::UnknownTemplate(other)),
    }
}

impl fmt::Display for SbeLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.price, self.quantity)
    }
}
