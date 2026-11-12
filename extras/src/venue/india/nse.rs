//! NSE cash-equity master parser (CM master, format `nse-cm-master-v1`).
//!
//! The NSE equity master is a wide CSV:
//!
//! ```text
//! SYMBOL,NAME OF COMPANY,SERIES,DATE OF LISTING,PAID UP VALUE,MARKET LOT,ISIN NUMBER,FACE VALUE
//! ```
//!
//! (Verified against the published `EQUITY_L.csv`: the ISIN column is
//! `ISIN NUMBER`, not `ISIN CODE`.) Only the columns the cash contract needs
//! are read; unknown trailing columns are tolerated. Required columns are
//! `SYMBOL`, `SERIES` and `ISIN NUMBER` (aliases accepted for older snapshots).
//!
//! `SERIES` semantics (exchange legend; second-review finding A):
//!
//! * `EQ` — normal equity segment. `Active`, `Standard` settlement.
//! * `BE` — trade-for-trade. `BE` is a *settlement regime*, not a
//!   suspension: the security trades with delivery from the specific contra
//!   trade. `Active`, `TradeForTrade`, no Z-category flag.
//! * `BZ` — trade-for-trade **with the Z-category designation** (added
//!   surveillance/compliance restrictions). `Active`, `TradeForTrade`, and
//!   `z_category: true` — preserved as its own attribute, never folded
//!   into the trading status.
//!
//! A row with any other series, an empty symbol/ISIN, or an invalid ISIN is
//! rejected — never dropped or silently downgraded.

use india_domain::{Isin, ListingStatus, SettlementRegime};

use super::csv::{column, optional_column, split_csv_file};

/// One validated NSE CM master record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NseCashRow {
    /// 1-based data line number in the raw file (header is line 1).
    pub line: usize,
    /// Trading symbol, scoped to `(NSE, cash)`.
    pub symbol: String,
    /// Company/ETF name.
    pub name: String,
    /// Raw `SERIES` value (`EQ`, `BE`, `BZ`).
    pub series: String,
    /// Validated ISIN (ISO 6166 check digit enforced by `india-domain`).
    pub isin: Isin,
    /// Market lot from `MARKET LOT`/`MARK LOT` when present; `1` otherwise.
    /// Cash-equity market lot is 1 after the exchange's reforms; the field is
    /// still read so a future non-1 file is honoured rather than flattened.
    pub market_lot: i64,
    /// Listing status implied by `SERIES`.
    pub status: ListingStatus,
    /// Settlement regime implied by `SERIES`.
    pub settlement_regime: SettlementRegime,
    /// Whether the security carries the Z-category designation.
    pub z_category: bool,
}

/// Parse the NSE CM master bytes already recorded by [`super::provenance`].
///
/// Fails closed on the first invalid row and reports its line number; the
/// caller records the failure rather than building a partial universe from
/// the rows before it.
pub fn parse_nse_cm_master(csv: &str) -> Result<Vec<NseCashRow>, anyhow::Error> {
    let (header, rows) = split_csv_file(csv)?;
    let symbol_col = column(&header, &["SYMBOL"])?;
    let series_col = column(&header, &["SERIES"])?;
    // The published master names this `ISIN NUMBER`.
    let isin_col = column(&header, &["ISIN NUMBER", "ISIN CODE", "ISIN", "ISIN_CODE"])?;
    let name_col = optional_column(&header, &["NAME OF COMPANY", "NAME", "COMPANY NAME"]);
    let lot_col = optional_column(&header, &["MARKET LOT", "MARK LOT"]);

    let mut out = Vec::with_capacity(rows.len());
    for (offset, row) in rows.iter().enumerate() {
        let line = offset + 2;
        let symbol = row[symbol_col].trim().to_owned();
        anyhow::ensure!(!symbol.is_empty(), "line {line}: empty SYMBOL");
        let series = row[series_col].trim().to_uppercase();
        let isin_text = row[isin_col].trim();
        anyhow::ensure!(
            !isin_text.is_empty(),
            "line {line}: empty ISIN for {symbol}"
        );
        let isin = Isin::new(isin_text)
            .map_err(|err| anyhow::anyhow!("line {line}: invalid ISIN for {symbol}: {err}"))?;
        let (status, settlement_regime, z_category) = match series.as_str() {
            "EQ" => (ListingStatus::Active, SettlementRegime::Standard, false),
            "BE" => (
                ListingStatus::Active,
                SettlementRegime::TradeForTrade,
                false,
            ),
            "BZ" => (ListingStatus::Active, SettlementRegime::TradeForTrade, true),
            other => anyhow::bail!("line {line}: SERIES {other} is outside the cash contract"),
        };
        let name = match name_col {
            Some(index) => row[index].trim().to_owned(),
            None => symbol.clone(),
        };
        let market_lot = match lot_col {
            Some(index) => row[index]
                .trim()
                .parse::<i64>()
                .map_err(|_| anyhow::anyhow!("line {line}: MARKET LOT is not an integer"))?,
            None => 1,
        };
        anyhow::ensure!(market_lot > 0, "line {line}: MARKET LOT must be positive");
        out.push(NseCashRow {
            line,
            symbol,
            name,
            series,
            isin,
            market_lot,
            status,
            settlement_regime,
            z_category,
        });
    }
    // One symbol, one record: a duplicate is a contradictory master, not two
    // listings to merge by guesswork.
    let mut seen = Vec::new();
    for row in &out {
        anyhow::ensure!(
            !seen.contains(&row.symbol),
            "line {}: duplicate SYMBOL {}",
            row.line,
            row.symbol
        );
        seen.push(row.symbol.clone());
    }
    Ok(out)
}
