//! NSE cash-equity master parser with explicit, versioned row outcomes.
//!
//! Scope is declared by the caller, not inferred from incoming rows. Missing
//! source facts are rejected rather than defaulted; series outside the cash
//! equity contract are excluded with a named rule.

use trading_core::{Isin, ListingStatus, SettlementRegime};

use crate::csv::{Limits, column, optional_column, parse_document};
use crate::observation::{Exclusion, Rejection, SnapshotScope, SourceBatch};

/// One accepted NSE cash-equity master record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NseCashRow {
    pub line: usize,
    pub symbol: String,
    pub name: String,
    pub series: String,
    pub isin: Isin,
    pub market_lot: i64,
    pub status: ListingStatus,
    pub settlement_regime: SettlementRegime,
    pub z_category: bool,
}

/// Parse NSE cash-equity master bytes.
///
/// `source_version` names the source contract that classifies rows; it is part
/// of the batch and must be recorded with the source hash.
pub fn parse_nse_cash_master(
    bytes: &[u8],
    source_version: &str,
    scope: SnapshotScope,
    limits: Limits,
) -> Result<SourceBatch<NseCashRow>, String> {
    let document = parse_document(bytes, limits).map_err(|error| error.to_string())?;
    let symbol_column = column(&document.header, &["SYMBOL"])?;
    let series_column = column(&document.header, &["SERIES"])?;
    let isin_column = column(
        &document.header,
        &["ISIN NUMBER", "ISIN CODE", "ISIN", "ISIN_CODE"],
    )?;
    let name_column = optional_column(
        &document.header,
        &["NAME OF COMPANY", "NAME", "COMPANY NAME"],
    )
    .ok_or_else(|| "NSE cash master has no company name column".to_owned())?;
    let lot_column = optional_column(&document.header, &["MARKET LOT", "MARK LOT"])
        .ok_or_else(|| "NSE cash master has no market lot column".to_owned())?;

    let mut batch = SourceBatch {
        source_id: "nse.cash_equity_master".to_owned(),
        source_version: source_version.to_owned(),
        scope,
        accepted: Vec::new(),
        excluded: Vec::new(),
        rejected: Vec::new(),
        physical_rows: document.rows.len(),
    };
    let mut symbols = std::collections::BTreeSet::new();
    let mut isins = std::collections::BTreeSet::new();
    for row in document.rows {
        let line = row.line;
        let symbol = row.fields[symbol_column].trim().to_owned();
        let series = row.fields[series_column].trim().to_uppercase();
        if symbol.is_empty() {
            batch.rejected.push(Rejection {
                line,
                reason: "empty SYMBOL".to_owned(),
            });
            continue;
        }
        let (status, settlement_regime, z_category) = match series.as_str() {
            "EQ" => (ListingStatus::Active, SettlementRegime::Standard, false),
            "BE" => (
                ListingStatus::Active,
                SettlementRegime::TradeForTrade,
                false,
            ),
            "BZ" => (ListingStatus::Active, SettlementRegime::TradeForTrade, true),
            other => {
                batch.excluded.push(Exclusion {
                    line,
                    rule: format!("series {other} outside nse_cash_equity scope"),
                });
                continue;
            }
        };
        let isin_text = row.fields[isin_column].trim();
        let isin = match Isin::new(isin_text) {
            Ok(value) => value,
            Err(error) => {
                batch.rejected.push(Rejection {
                    line,
                    reason: format!("invalid ISIN {isin_text}: {error}"),
                });
                continue;
            }
        };
        let name = row.fields[name_column].trim().to_owned();
        if name.is_empty() {
            batch.rejected.push(Rejection {
                line,
                reason: "empty company name; no fallback identity is inferred".to_owned(),
            });
            continue;
        }
        let market_lot = match row.fields[lot_column].trim().parse::<i64>() {
            Ok(value) if value > 0 => value,
            Ok(value) => {
                batch.rejected.push(Rejection {
                    line,
                    reason: format!("non-positive MARKET LOT {value}"),
                });
                continue;
            }
            Err(_) => {
                batch.rejected.push(Rejection {
                    line,
                    reason: "MARKET LOT is not an integer".to_owned(),
                });
                continue;
            }
        };
        let isin_key = isin.as_str().to_owned();
        if symbols.contains(&symbol) || isins.contains(&isin_key) {
            batch.rejected.push(Rejection {
                line,
                reason: format!("duplicate symbol or ISIN for {symbol} / {isin_key}"),
            });
            continue;
        }
        symbols.insert(symbol.clone());
        isins.insert(isin_key);
        batch.accepted.push(NseCashRow {
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
    Ok(batch)
}
