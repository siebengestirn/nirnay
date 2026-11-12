//! Minimal RFC-4180 CSV reader for exchange master files.
//!
//! Exchange masters do not need a general CSV crate: fields are either bare
//! tokens or double-quoted strings that may contain commas. This reader
//! handles line endings, quoted commas and escaped quotes, and rejects rows
//! whose field count differs from the header instead of guessing.

/// Split one record into fields.
///
/// Rejects an unterminated quote. An empty record yields one empty field.
pub fn split_csv_record(record: &str) -> Result<Vec<String>, anyhow::Error> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = record.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
        } else if ch == '"' {
            if field.is_empty() {
                quoted = true;
            } else {
                anyhow::bail!("unexpected quote in unquoted csv field");
            }
        } else if ch == ',' {
            fields.push(field.trim().to_owned());
            field = String::new();
        } else {
            field.push(ch);
        }
    }
    anyhow::ensure!(!quoted, "unterminated quote in csv record");
    fields.push(field.trim().to_owned());
    Ok(fields)
}

/// Split a whole file into `(header, rows)` with trimmed, upper-cased header
/// names for lookup. Blank lines are skipped.
pub fn split_csv_file(csv: &str) -> Result<(Vec<String>, Vec<Vec<String>>), anyhow::Error> {
    let mut lines = csv.lines().filter(|line| !line.trim().is_empty());
    let header_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("csv file has no header"))?;
    let header = split_csv_record(header_line)?
        .into_iter()
        .map(|name| name.trim().to_uppercase())
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for line in lines {
        let row = split_csv_record(line)?;
        anyhow::ensure!(
            row.len() == header.len(),
            "csv row has {} fields, header has {}",
            row.len(),
            header.len()
        );
        rows.push(row);
    }
    Ok((header, rows))
}

/// Resolve a required column by any of `candidates` (already upper-cased).
pub fn column(header: &[String], candidates: &[&str]) -> Result<usize, anyhow::Error> {
    for candidate in candidates {
        if let Some(index) = header.iter().position(|name| name == candidate) {
            return Ok(index);
        }
    }
    anyhow::bail!(
        "csv header is missing a required column (tried {})",
        candidates.join("/")
    );
}

/// Optional column lookup; `None` when absent.
#[must_use]
pub fn optional_column(header: &[String], candidates: &[&str]) -> Option<usize> {
    candidates
        .iter()
        .find_map(|candidate| header.iter().position(|name| name == candidate))
}
