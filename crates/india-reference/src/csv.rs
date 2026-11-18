//! Bounded CSV reader for exchange master files.
//!
//! This is a logical-record parser, not a line splitter: a quoted field may
//! contain CRLF, LF or embedded commas. Every input is limited by byte count,
//! row count, field count and field length so a corrupt source cannot exhaust
//! memory. Malformed quoting is rejected, never guessed.

use thiserror::Error;

/// Explicit ingestion limits. These are source-file limits, not business rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: usize,
    pub max_rows: usize,
    pub max_fields: usize,
    pub max_field_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 64 * 1024 * 1024,
            max_rows: 500_000,
            max_fields: 128,
            max_field_bytes: 4096,
        }
    }
}

/// A parsed CSV record with its starting physical line number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvRow {
    pub line: usize,
    pub fields: Vec<String>,
}

/// A parsed CSV document: header plus data records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvDocument {
    pub header: Vec<String>,
    pub rows: Vec<CsvRow>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CsvError {
    #[error("source exceeds {limit} byte file limit")]
    FileTooLarge { limit: usize },
    #[error("source exceeds {limit} row limit")]
    TooManyRows { limit: usize },
    #[error("record at line {line} exceeds {limit} fields")]
    TooManyFields { line: usize, limit: usize },
    #[error("field at line {line} exceeds {limit} bytes")]
    FieldTooLarge { line: usize, limit: usize },
    #[error("unterminated quoted field at line {line}")]
    UnterminatedQuote { line: usize },
    #[error("unexpected character after closing quote at line {line}")]
    TrailingQuoteCharacter { line: usize },
    #[error("unexpected quote in unquoted field at line {line}")]
    UnexpectedQuote { line: usize },
    #[error("source has no header record")]
    MissingHeader,
    #[error("record at line {line} has {actual} fields; header has {expected}")]
    FieldCountMismatch {
        line: usize,
        actual: usize,
        expected: usize,
    },
    #[error("duplicate header column `{name}`")]
    DuplicateHeader { name: String },
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

/// Parse a bounded source into a document. The entire source must be valid
/// UTF-8; invalid encoding fails closed before any field interpretation.
pub fn parse_document(bytes: &[u8], limits: Limits) -> Result<CsvDocument, CsvError> {
    let bytes = strip_bom(bytes);
    if bytes.len() > limits.max_file_bytes {
        return Err(CsvError::FileTooLarge {
            limit: limits.max_file_bytes,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| CsvError::UnexpectedQuote { line: 0 })?;
    let mut records = Vec::new();
    let mut field = String::new();
    let mut fields = Vec::new();
    let mut quoted = false;
    let mut quote_closed = false;
    let mut line = 1_usize;
    let mut record_line = 1_usize;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    push_char(&mut field, '"', line, limits.max_field_bytes)?;
                } else {
                    quoted = false;
                    quote_closed = true;
                }
            } else {
                if ch == '\n' {
                    line += 1;
                }
                push_char(&mut field, ch, line, limits.max_field_bytes)?;
            }
            continue;
        }
        if quote_closed && !matches!(ch, ',' | '\r' | '\n') {
            return Err(CsvError::TrailingQuoteCharacter { line });
        }
        match ch {
            '"' if quote_closed => {
                return Err(CsvError::TrailingQuoteCharacter { line });
            }
            '"' if field.is_empty() => {
                quoted = true;
                quote_closed = false;
            }
            '"' => return Err(CsvError::UnexpectedQuote { line }),
            ',' => {
                fields.push(std::mem::take(&mut field));
                quote_closed = false;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                finish_record(
                    &mut fields,
                    &mut field,
                    record_line,
                    &mut records,
                    limits.max_fields,
                )?;
                line += 1;
                record_line = line;
                quote_closed = false;
            }
            '\n' => {
                finish_record(
                    &mut fields,
                    &mut field,
                    record_line,
                    &mut records,
                    limits.max_fields,
                )?;
                line += 1;
                record_line = line;
                quote_closed = false;
            }
            other => {
                push_char(&mut field, other, line, limits.max_field_bytes)?;
                quote_closed = false;
            }
        }
    }
    if quoted {
        return Err(CsvError::UnterminatedQuote { line });
    }
    if !field.is_empty() || !fields.is_empty() {
        finish_record(
            &mut fields,
            &mut field,
            record_line,
            &mut records,
            limits.max_fields,
        )?;
    }
    if records.len() > limits.max_rows.saturating_add(1) {
        return Err(CsvError::TooManyRows {
            limit: limits.max_rows,
        });
    }
    let header_record = records.first().ok_or(CsvError::MissingHeader)?;
    let header = header_record
        .fields
        .iter()
        .map(|value| value.trim().to_uppercase())
        .collect::<Vec<_>>();
    let mut seen = std::collections::BTreeSet::new();
    for name in &header {
        if !seen.insert(name.clone()) {
            return Err(CsvError::DuplicateHeader { name: name.clone() });
        }
    }
    let mut rows = Vec::with_capacity(records.len().saturating_sub(1));
    for record in records.into_iter().skip(1) {
        if record.fields.len() != header.len() {
            return Err(CsvError::FieldCountMismatch {
                line: record.line,
                actual: record.fields.len(),
                expected: header.len(),
            });
        }
        if rows.len() >= limits.max_rows {
            return Err(CsvError::TooManyRows {
                limit: limits.max_rows,
            });
        }
        rows.push(record);
    }
    Ok(CsvDocument { header, rows })
}

fn push_char(field: &mut String, value: char, line: usize, limit: usize) -> Result<(), CsvError> {
    if field.len().saturating_add(value.len_utf8()) > limit {
        return Err(CsvError::FieldTooLarge { line, limit });
    }
    field.push(value);
    Ok(())
}

fn finish_record(
    fields: &mut Vec<String>,
    field: &mut String,
    line: usize,
    records: &mut Vec<CsvRow>,
    max_fields: usize,
) -> Result<(), CsvError> {
    fields.push(std::mem::take(field));
    if fields.len() > max_fields {
        return Err(CsvError::TooManyFields {
            line,
            limit: max_fields,
        });
    }
    records.push(CsvRow {
        line,
        fields: std::mem::take(fields),
    });
    Ok(())
}

/// Resolve a required column by uppercase name.
pub fn column(header: &[String], candidates: &[&str]) -> Result<usize, String> {
    for candidate in candidates {
        if let Some(index) = header.iter().position(|name| name == candidate) {
            return Ok(index);
        }
    }
    Err(format!(
        "missing required column (tried {})",
        candidates.join("/")
    ))
}

/// Resolve an optional column by uppercase name.
#[must_use]
pub fn optional_column(header: &[String], candidates: &[&str]) -> Option<usize> {
    candidates
        .iter()
        .find_map(|candidate| header.iter().position(|name| name == candidate))
}
