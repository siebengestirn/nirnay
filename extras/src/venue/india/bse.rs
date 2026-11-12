//! BSE security-master parser (format `bse-security-master-v1`).
//!
//! The BSE scrip master publishes one row per security with the fields below
//! (column set per the BSE scrip-master specification cited in the P1-03
//! third review; extra trailing columns are tolerated):
//!
//! ```text
//! Scrip Code,Instrument Code,Scrip Name,Group Name,ISIN CODE,New Tick Size,
//! Market Lot,Security Type Flag
//! ```
//!
//! | Column               | Meaning                       | Carried as              |
//! | -------------------- | ----------------------------- | ----------------------- |
//! | `Scrip Code`         | Numeric security identifier   | `scrip_code` (token)    |
//! | `Instrument Code`    | Security symbol               | `symbol`                |
//! | `Scrip Name`         | Full security name            | `name`                  |
//! | `Group Name`         | Settlement group              | `settlement_group`      |
//! | `ISIN CODE`          | Instrument identity           | `isin`                  |
//! | `New Tick Size`      | Tick size in **rupees** (`FLOAT[3,2]`, e.g. `0.05`) | `tick_paise` (exact paise) |
//! | `Market Lot`         | Lot size of the security      | `market_lot`            |
//! | `Security Type Flag` | Security category             | `security_type`         |
//!
//! * `Group Name`, `New Tick Size` and `Market Lot` are trading information
//!   and are retained end-to-end, never defaulted. Losing one would silently
//!   change the trading contract, so missing columns, a missing/unknown
//!   group, a non-positive Market Lot or a non-positive tick are parse
//!   errors.
//! * `New Tick Size` is a decimal rupee amount (BSE field type
//!   `FLOAT[3,2]`): `0.05` is five paise, `5.00` is five hundred paise. It is
//!   converted to paise by integer arithmetic, never via floating point.
//! * `Group Name` determines settlement. BSE trade-to-trade groups (`T`,
//!   `MT`, `TS`, `IT`, `Z`, `R`, `ZP`, `P`, `Y`, `ZY`, `XT`) map to
//!   [`india_domain::SettlementRegime::TradeForTrade`] with no netting; `Z*`
//!   groups also carry the Z-category surveillance designation. Groups
//!   outside the classified set fail closed rather than being assumed to be
//!   rolling-settlement.
//! * `Security Type Flag` is validated. Only `EQ` (equity) is inside this
//!   contract; `MF` (mutual funds), `DB` (debt) and `GS` (government
//!   securities) are rejected explicitly because their domain contracts do
//!   not exist yet. An unknown flag is also rejected — never guessed.
//! * `STATUS` is optional: `ACTIVE` / `SUSPENDED` / `DELISTED`; absent
//!   means `Active`. Unknown values are rejected.
//!
//! A row is never dropped or silently downgraded: the whole file fails on
//! the first invalid row, and the caller records the failure instead of
//! building a partial universe.

use india_domain::{Isin, ListingStatus, SettlementRegime};

use super::csv::{column, optional_column, split_csv_file};

/// BSE `Security Type Flag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BseSecurityType {
    /// `EQ` — equity. The only category this contract supports.
    Equity,
    /// `MF` — mutual fund. Rejected: no domain contract yet.
    MutualFund,
    /// `DB` — debt. Rejected: no domain contract yet.
    Debt,
    /// `GS` — government security. Rejected: no domain contract yet.
    GovernmentSecurity,
}

impl BseSecurityType {
    /// Parse the exchange flag. Unknown values fail closed.
    fn parse(flag: &str) -> Result<Self, anyhow::Error> {
        match flag {
            "EQ" => Ok(Self::Equity),
            "MF" => Ok(Self::MutualFund),
            "DB" => Ok(Self::Debt),
            "GS" => Ok(Self::GovernmentSecurity),
            other => anyhow::bail!("unknown Security Type Flag {other}"),
        }
    }

    /// The exchange code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Equity => "EQ",
            Self::MutualFund => "MF",
            Self::Debt => "DB",
            Self::GovernmentSecurity => "GS",
        }
    }

    /// Human-readable category.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Equity => "equity",
            Self::MutualFund => "mutual fund",
            Self::Debt => "debt",
            Self::GovernmentSecurity => "government security",
        }
    }
}

/// Settlement meaning of a BSE `Group Name`.
///
/// Tradability and settlement are different questions: every classified
/// group is tradable, but a trade-to-trade group settles each trade
/// individually and therefore cannot be squared off intraday.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BseGroupClass {
    /// Settlement regime the group imposes.
    pub settlement_regime: SettlementRegime,
    /// Z-category surveillance designation carried by `Z`, `ZP` and `ZY`.
    pub z_category: bool,
}

/// BSE groups that settle trade-to-trade (no netting), per the BSE
/// settlement-group classification cited in the P1-03 fourth review.
const BSE_TRADE_FOR_TRADE_GROUPS: &[&str] =
    &["T", "MT", "TS", "IT", "Z", "R", "ZP", "P", "Y", "ZY", "XT"];

/// BSE groups that use normal rolling settlement. Groups outside this list
/// and the trade-to-trade list are rejected: assuming rolling settlement for
/// an unclassified group could silently permit intraday trading in a
/// no-netting security.
const BSE_STANDARD_SETTLEMENT_GROUPS: &[&str] = &["A", "B", "M", "MS", "S", "X", "XC", "XD"];

/// Classify a BSE settlement group. Unknown groups fail closed.
pub fn classify_bse_group(group: &str) -> Result<BseGroupClass, anyhow::Error> {
    let code = group.trim().to_uppercase();
    anyhow::ensure!(!code.is_empty(), "empty Group Name");
    let z_category = matches!(code.as_str(), "Z" | "ZP" | "ZY");
    if BSE_TRADE_FOR_TRADE_GROUPS.contains(&code.as_str()) {
        return Ok(BseGroupClass {
            settlement_regime: SettlementRegime::TradeForTrade,
            z_category,
        });
    }
    if BSE_STANDARD_SETTLEMENT_GROUPS.contains(&code.as_str()) {
        return Ok(BseGroupClass {
            settlement_regime: SettlementRegime::Standard,
            z_category,
        });
    }
    anyhow::bail!("unknown BSE Group Name {code}; refusing to guess settlement eligibility")
}

/// Parse a BSE `FLOAT[3,2]` rupee amount into paise using integer
/// arithmetic only.
///
/// `FLOAT[3,2]` is a decimal with at most three digits and two fractional
/// digits, so `0.05` becomes `5` paise and `5.00` becomes `500`. A value
/// with more than two fractional digits, a negative/zero amount or a
/// non-numeric value is rejected; floating point is never involved.
pub fn parse_rupees_to_paise(text: &str) -> Result<i64, anyhow::Error> {
    let value = text.trim();
    anyhow::ensure!(!value.is_empty(), "empty rupee amount");
    let (whole, fractional) = match value.split_once('.') {
        Some((whole, fractional)) => (whole, Some(fractional)),
        None => (value, None),
    };
    anyhow::ensure!(
        !whole.is_empty() && whole.bytes().all(|byte| byte.is_ascii_digit()),
        "rupee amount {value} has no valid integer part"
    );
    if let Some(fractional) = fractional {
        anyhow::ensure!(
            fractional.len() <= 2 && fractional.bytes().all(|byte| byte.is_ascii_digit()),
            "rupee amount {value} must have at most two fractional digits"
        );
    }
    let digits = whole.len() + fractional.map_or(0, str::len);
    anyhow::ensure!(digits <= 3, "rupee amount {value} exceeds FLOAT[3,2]");
    let whole: i64 = whole
        .parse()
        .map_err(|_| anyhow::anyhow!("rupee amount {value} is not a number"))?;
    let paise = match fractional {
        None => 0,
        Some("") => 0,
        Some(fraction) if fraction.len() == 1 => {
            fraction
                .parse::<i64>()
                .map_err(|_| anyhow::anyhow!("rupee amount {value} is not a number"))?
                * 10
        }
        Some(fraction) if fraction.len() == 2 => fraction
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("rupee amount {value} is not a number"))?,
        Some(_) => unreachable!("fractional length checked above"),
    };
    let total = whole
        .checked_mul(100)
        .and_then(|value| value.checked_add(paise))
        .ok_or_else(|| anyhow::anyhow!("rupee amount {value} overflows paise"))?;
    anyhow::ensure!(total > 0, "tick size must be positive");
    Ok(total)
}

/// One validated BSE security-master record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BseCashRow {
    /// 1-based data line number in the raw file (header is line 1).
    pub line: usize,
    /// BSE `Scrip Code`: numeric security identifier, carried as the
    /// listing token. Never identity.
    pub scrip_code: String,
    /// BSE `Instrument Code`: the security symbol.
    pub symbol: String,
    /// BSE `Scrip Name`: full security name.
    pub name: String,
    /// BSE `Group Name`: settlement group (required, trading information).
    pub settlement_group: String,
    /// Settlement classification derived from `Group Name`.
    pub group_class: BseGroupClass,
    /// Validated ISIN (ISO 6166 check digit enforced by `india-domain`).
    pub isin: Isin,
    /// BSE `New Tick Size` for this scrip, converted from decimal rupees to
    /// exact paise (trading information, retained as the assigned tick).
    pub tick_paise: i64,
    /// BSE `Market Lot`: the lot size of the security. Never defaulted.
    pub market_lot: i64,
    /// BSE `Security Type Flag`. Always [`BseSecurityType::Equity`] after a
    /// successful parse; other categories are rejected.
    pub security_type: BseSecurityType,
    /// Listing status from the optional `STATUS` column.
    pub status: ListingStatus,
}

/// Parse the BSE security-master bytes already recorded by
/// [`super::provenance`].
///
/// Fails closed on the first invalid row and reports its line number.
pub fn parse_bse_master(csv: &str) -> Result<Vec<BseCashRow>, anyhow::Error> {
    let (header, rows) = split_csv_file(csv)?;
    let scrip_code_col = column(&header, &["SCRIP CODE"])?;
    let symbol_col = column(&header, &["INSTRUMENT CODE"])?;
    let name_col = column(&header, &["SCRIP NAME"])?;
    let isin_col = column(&header, &["ISIN CODE", "ISIN"])?;
    let tick_col = column(&header, &["NEW TICK SIZE"])?;
    let lot_col = column(&header, &["MARKET LOT"])?;
    let type_col = column(
        &header,
        &["SECURITY TYPE FLAG", "SECURITY TYPE", "SECURITY_TYPE_FLAG"],
    )?;
    let group_col = column(&header, &["GROUP NAME"])?;
    let status_col = optional_column(&header, &["STATUS"]);

    let mut out = Vec::with_capacity(rows.len());
    for (offset, row) in rows.iter().enumerate() {
        let line = offset + 2;
        let scrip_code = row[scrip_code_col].trim().to_owned();
        anyhow::ensure!(!scrip_code.is_empty(), "line {line}: empty Scrip Code");
        let symbol = row[symbol_col].trim().to_owned();
        anyhow::ensure!(!symbol.is_empty(), "line {line}: empty Instrument Code");
        let name = row[name_col].trim().to_owned();
        anyhow::ensure!(!name.is_empty(), "line {line}: empty Scrip Name");
        let isin_text = row[isin_col].trim();
        anyhow::ensure!(
            !isin_text.is_empty(),
            "line {line}: empty ISIN for {symbol}"
        );
        let isin = Isin::new(isin_text)
            .map_err(|err| anyhow::anyhow!("line {line}: invalid ISIN for {symbol}: {err}"))?;
        let settlement_group = row[group_col].trim().to_owned();
        anyhow::ensure!(
            !settlement_group.is_empty(),
            "line {line}: empty Group Name for {symbol}"
        );
        let group_class = classify_bse_group(&settlement_group)
            .map_err(|err| anyhow::anyhow!("line {line}: {err}"))?;
        let tick_paise = parse_rupees_to_paise(row[tick_col].trim())
            .map_err(|err| anyhow::anyhow!("line {line}: New Tick Size: {err}"))?;
        let market_lot = row[lot_col]
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("line {line}: Market Lot is not an integer"))?;
        anyhow::ensure!(market_lot > 0, "line {line}: Market Lot must be positive");
        let flag = row[type_col].trim().to_uppercase();
        anyhow::ensure!(
            !flag.is_empty(),
            "line {line}: empty Security Type Flag for {symbol}"
        );
        let security_type =
            BseSecurityType::parse(&flag).map_err(|err| anyhow::anyhow!("line {line}: {err}"))?;
        anyhow::ensure!(
            security_type == BseSecurityType::Equity,
            "line {line}: Security Type Flag {} ({}) is outside the cash-equity contract",
            security_type.code(),
            security_type.label()
        );
        let status_text = match status_col {
            Some(index) => row[index].trim().to_uppercase(),
            None => "ACTIVE".to_owned(),
        };
        let status = match status_text.as_str() {
            "ACTIVE" => ListingStatus::Active,
            "SUSPENDED" => ListingStatus::Suspended,
            "DELISTED" => ListingStatus::Delisted,
            other => anyhow::bail!("line {line}: status {other} is not recognised"),
        };
        out.push(BseCashRow {
            line,
            scrip_code,
            symbol,
            name,
            settlement_group,
            group_class,
            isin,
            tick_paise,
            market_lot,
            security_type,
            status,
        });
    }
    // One symbol, one record per master.
    let mut seen = Vec::new();
    for row in &out {
        anyhow::ensure!(
            !seen.contains(&row.symbol),
            "line {}: duplicate symbol {}",
            row.line,
            row.symbol
        );
        seen.push(row.symbol.clone());
    }
    Ok(out)
}
