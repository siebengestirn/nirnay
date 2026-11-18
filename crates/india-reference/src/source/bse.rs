//! BSE security-master parser with versioned settlement classification.
//!
//! Trading eligibility is never inferred from an unknown group. The group
//! classification is selected by the declared source version and must be
//! validated against actual dated files before that version is supported.

use trading_core::{Isin, ListingStatus, SettlementRegime};

use crate::csv::{Limits, column, parse_document};
use crate::observation::{Exclusion, Rejection, SnapshotScope, SourceBatch};

/// Settlement classification for a BSE group in one source version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BseGroupClass {
    pub settlement_regime: SettlementRegime,
    pub z_category: bool,
}

/// Versioned group classification. Unknown source versions are rejected.
pub fn bse_group_classes(
    source_version: &str,
) -> Result<Vec<(&'static str, BseGroupClass)>, String> {
    match source_version {
        "bse-security-master-v1" => {
            let trade = ["T", "MT", "TS", "IT", "Z", "R", "ZP", "P", "Y", "ZY", "XT"];
            let standard = ["A", "B", "M", "MS", "S", "X", "XC", "XD"];
            let mut rows = Vec::new();
            for group in trade {
                rows.push((
                    group,
                    BseGroupClass {
                        settlement_regime: SettlementRegime::TradeForTrade,
                        z_category: matches!(group, "Z" | "ZP" | "ZY"),
                    },
                ));
            }
            for group in standard {
                rows.push((
                    group,
                    BseGroupClass {
                        settlement_regime: SettlementRegime::Standard,
                        z_category: false,
                    },
                ));
            }
            Ok(rows)
        }
        other => Err(format!("unsupported BSE source version {other}")),
    }
}

/// One accepted BSE cash-market record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BseCashRow {
    pub line: usize,
    pub scrip_code: String,
    pub symbol: String,
    pub name: String,
    pub isin: Isin,
    pub settlement_group: String,
    pub group_class: BseGroupClass,
    pub tick_paise: i64,
    pub market_lot: i64,
    pub status: ListingStatus,
}

/// Parse BSE security-master bytes under a declared source version.
pub fn parse_bse_master(
    bytes: &[u8],
    source_version: &str,
    scope: SnapshotScope,
    limits: Limits,
) -> Result<SourceBatch<BseCashRow>, String> {
    let classes = bse_group_classes(source_version)?;
    let document = parse_document(bytes, limits).map_err(|error| error.to_string())?;
    let scrip_column = column(&document.header, &["SCRIP CODE"])?;
    let symbol_column = column(&document.header, &["INSTRUMENT CODE"])?;
    let name_column = column(&document.header, &["SCRIP NAME"])?;
    let isin_column = column(&document.header, &["ISIN CODE", "ISIN"])?;
    let tick_column = column(&document.header, &["NEW TICK SIZE"])?;
    let lot_column = column(&document.header, &["MARKET LOT"])?;
    let type_column = column(
        &document.header,
        &["SECURITY TYPE FLAG", "SECURITY TYPE", "SECURITY_TYPE_FLAG"],
    )?;
    let group_column = column(&document.header, &["GROUP NAME"])?;
    let status_column = column(&document.header, &["STATUS"])?;

    let mut batch = SourceBatch {
        source_id: "bse.cash_security_master".to_owned(),
        source_version: source_version.to_owned(),
        scope,
        accepted: Vec::new(),
        excluded: Vec::new(),
        rejected: Vec::new(),
        physical_rows: document.rows.len(),
    };
    let mut scrips = std::collections::BTreeSet::new();
    let mut isins = std::collections::BTreeSet::new();
    for row in document.rows {
        let line = row.line;
        let scrip_code = row.fields[scrip_column].trim().to_owned();
        let symbol = row.fields[symbol_column].trim().to_owned();
        if scrip_code.is_empty() || symbol.is_empty() {
            batch.rejected.push(Rejection {
                line,
                reason: "empty SCRIP CODE or INSTRUMENT CODE".to_owned(),
            });
            continue;
        }
        if !row.fields[type_column].trim().eq_ignore_ascii_case("EQ") {
            batch.excluded.push(Exclusion {
                line,
                rule: format!(
                    "security type {} outside bse_cash scope",
                    row.fields[type_column].trim()
                ),
            });
            continue;
        }
        let group = row.fields[group_column].trim().to_uppercase();
        let group_class = match classes.iter().find(|(name, _)| *name == group) {
            Some((_, class)) => *class,
            None => {
                batch.rejected.push(Rejection {
                    line,
                    reason: format!("unclassified BSE group {group}; eligibility refused"),
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
                reason: "empty SCRIP NAME; no fallback identity is inferred".to_owned(),
            });
            continue;
        }
        let tick_paise = match market_types::fixed::parse_scaled(row.fields[tick_column].trim(), 2)
        {
            Ok(value) if value > 0 => value,
            _ => {
                batch.rejected.push(Rejection {
                    line,
                    reason: "NEW TICK SIZE is not a positive paise value".to_owned(),
                });
                continue;
            }
        };
        let market_lot = match row.fields[lot_column].trim().parse::<i64>() {
            Ok(value) if value > 0 => value,
            _ => {
                batch.rejected.push(Rejection {
                    line,
                    reason: "MARKET LOT is not a positive integer".to_owned(),
                });
                continue;
            }
        };
        let status = match row.fields[status_column].trim().to_uppercase().as_str() {
            "A" | "ACTIVE" => ListingStatus::Active,
            other => {
                batch.rejected.push(Rejection {
                    line,
                    reason: format!("unsupported STATUS {other}"),
                });
                continue;
            }
        };
        let isin_key = isin.as_str().to_owned();
        if scrips.contains(&scrip_code) || isins.contains(&isin_key) {
            batch.rejected.push(Rejection {
                line,
                reason: format!("duplicate scrip code or ISIN for {scrip_code} / {isin_key}"),
            });
            continue;
        }
        scrips.insert(scrip_code.clone());
        isins.insert(isin_key);
        batch.accepted.push(BseCashRow {
            line,
            scrip_code,
            symbol,
            name,
            isin,
            settlement_group: group,
            group_class,
            tick_paise,
            market_lot,
            status,
        });
    }
    Ok(batch)
}
