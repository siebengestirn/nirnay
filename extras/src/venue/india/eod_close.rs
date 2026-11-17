//! NSE end-of-day close dataset (format `nse-eod-close-v1`).
//!
//! The monthly NSE tick rule fixes an instrument's tick for month M from the
//! *close of the last trading session of month M-1*. A close is only evidence
//! if it is a record of the archived source, so this module parses that
//! source rather than trusting a bare number:
//!
//! ```text
//! DATE,SYMBOL,ISIN,CLOSE_PAISE
//! 2025-05-30,RELIANCE,INE002A01018,30000
//! ```
//!
//! `DATE` is an ISO date, `ISIN` identifies the instrument (the canonical
//! identity, never the symbol), and `CLOSE_PAISE` is the exact close in
//! paise. A caller that holds a genuine archive hash but supplies a
//! different instrument, date or price therefore has no matching row and
//! fails closed.

use india_domain::Isin;
use jiff::civil::Date;

use super::csv::{column, split_csv_file};

/// One parsed end-of-day close record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EodCloseRow {
    /// Trading date of the close.
    pub date: Date,
    /// Exchange symbol (lookup attribute, never identity).
    pub symbol: String,
    /// Instrument identity.
    pub isin: Isin,
    /// Exact close in paise.
    pub close_paise: i64,
}

/// Parse an archived end-of-day close file. Any malformed row is an error:
/// a partially parsed close table is not evidence.
pub fn parse_eod_close_csv(csv: &str) -> Result<Vec<EodCloseRow>, anyhow::Error> {
    let (header, rows) = split_csv_file(csv)?;
    let date_col = column(&header, &["DATE", "TRADE_DATE"])?;
    let symbol_col = column(&header, &["SYMBOL"])?;
    let isin_col = column(&header, &["ISIN", "ISIN NUMBER"])?;
    let close_col = column(&header, &["CLOSE_PAISE", "CLOSE"])?;
    let mut parsed = Vec::with_capacity(rows.len());
    // One close per instrument per trading date. A second row for the same
    // (ISIN, date) is contradictory source data, not a second observation:
    // accepting it would let a caller pick whichever price it prefers.
    let mut seen: std::collections::BTreeSet<(String, Date)> = std::collections::BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let line = index + 2;
        let date = Date::strptime("%Y-%m-%d", &row[date_col]).map_err(|_| {
            anyhow::anyhow!("line {line}: invalid ISO close date `{}`", row[date_col])
        })?;
        let symbol = row[symbol_col].trim().to_owned();
        anyhow::ensure!(!symbol.is_empty(), "line {line}: empty Symbol");
        let isin = Isin::new(row[isin_col].clone())
            .map_err(|err| anyhow::anyhow!("line {line}: invalid ISIN: {err}"))?;
        anyhow::ensure!(
            seen.insert((isin.as_str().to_owned(), date)),
            "line {line}: duplicate close record for {} on {date}",
            isin.as_str()
        );
        let close_paise = row[close_col]
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("line {line}: Close is not an integer number of paise"))?;
        anyhow::ensure!(close_paise >= 0, "line {line}: close must be non-negative");
        parsed.push(EodCloseRow {
            date,
            symbol,
            isin,
            close_paise,
        });
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::parse_eod_close_csv;

    #[test]
    fn parses_one_record_per_instrument_and_date() {
        let csv = "DATE,SYMBOL,ISIN,CLOSE_PAISE\n\
2025-05-30,RELIANCE,INE002A01018,30000\n\
2025-05-30,HDFCBANK,INE040A01026,150000\n";
        let rows = parse_eod_close_csv(csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].close_paise, 30_000);
        assert_eq!(rows[1].isin.as_str(), "INE040A01026");
    }

    #[test]
    fn duplicate_isin_date_is_rejected_even_with_a_conflicting_price() {
        // Both rows are individually valid and only one matches a proposed
        // observation. The source is contradictory, so the whole file is
        // rejected rather than letting the caller choose a price.
        let csv = "DATE,SYMBOL,ISIN,CLOSE_PAISE\n\
2025-05-30,RELIANCE,INE002A01018,30000\n\
2025-05-30,RELIANCE,INE002A01018,30100\n";
        let error = parse_eod_close_csv(csv).expect_err("a duplicate close is not evidence");
        assert!(
            error.to_string().contains("duplicate close record"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn the_same_isin_on_another_date_is_a_second_observation() {
        let csv = "DATE,SYMBOL,ISIN,CLOSE_PAISE\n\
2025-05-29,RELIANCE,INE002A01018,30000\n\
2025-05-30,RELIANCE,INE002A01018,30100\n";
        assert_eq!(parse_eod_close_csv(csv).unwrap().len(), 2);
    }
}
