//! Fail-closed pre-trade risk evaluation.
//!
//! Derived from the internal Nirnay `crates/nirnay-core/src/risk.rs` at source snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! The policy is evaluated against the canonical India instrument model:
//! listing activity comes from `india-domain`, quantities are non-negative
//! lots, and every cash limit is exact paise. A policy that fails validation
//! is never evaluated; an evaluation returns every check so a rejection can
//! be explained rather than inferred.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use trading_core::{
    AccountId, AlgoId, DomainError, FeedHealth, Instrument, InstrumentId, LogicalNanos,
    OrderIntent, Paise, Quantity, QuoteSnapshot, RiskAssessment, RiskCheck, RiskCheckResult, Side,
    StrategyId, TradeDate, Venue,
};

/// Effective risk policy. All limits are mandatory; validation rejects an
/// empty allowlist, a zero limit or an impossible collar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskPolicy {
    pub version: String,
    pub allowed_accounts: BTreeSet<AccountId>,
    pub allowed_strategies: BTreeSet<StrategyId>,
    pub allowed_algos: BTreeSet<AlgoId>,
    pub allowed_instruments: BTreeSet<InstrumentId>,
    pub max_order_quantity: Quantity,
    pub max_order_notional_paise: Paise,
    pub max_position_quantity: Quantity,
    pub max_gross_exposure_paise: Paise,
    pub max_daily_loss_paise: Paise,
    pub max_open_orders: u32,
    pub max_orders_per_second: u32,
    pub max_price_deviation_ppm: u32,
    pub max_quote_age_ns: u64,
    pub allow_short: bool,
}

impl RiskPolicy {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.version.trim().is_empty()
            || self.allowed_accounts.is_empty()
            || self.allowed_strategies.is_empty()
            || self.allowed_algos.is_empty()
            || self.allowed_instruments.is_empty()
            || self.max_order_quantity.is_zero()
            || self.max_order_notional_paise.0 <= 0
            || self.max_position_quantity.is_zero()
            || self.max_gross_exposure_paise.0 <= 0
            || self.max_daily_loss_paise.0 <= 0
            || self.max_open_orders == 0
            || self.max_orders_per_second == 0
            || self.max_price_deviation_ppm > 1_000_000
            || self.max_quote_age_ns == 0
        {
            return Err(DomainError::InvalidInstrument(
                "risk policy has invalid or empty limits".to_owned(),
            ));
        }
        Ok(())
    }
}

/// A risk policy validated for evaluation. There is exactly one source of
/// a `ValidatedRiskPolicy`: [`ValidatedRiskPolicy::from_policy`], which
/// runs [`RiskPolicy::validate`] first. An unvalidated policy can never
/// reach [`evaluate_risk`]; deserialization is equally unable to skip the
/// check because construction, not serde, is the only entry point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedRiskPolicy {
    policy: RiskPolicy,
}

impl ValidatedRiskPolicy {
    /// Validate the policy and construct a value that permits evaluation. Rejects
    /// empty allowlists, zero limits, and impossible collars with the same
    /// typed error as [`RiskPolicy::validate`].
    pub fn from_policy(policy: &RiskPolicy) -> Result<Self, DomainError> {
        policy.validate()?;
        Ok(Self {
            policy: policy.clone(),
        })
    }
}

impl std::ops::Deref for ValidatedRiskPolicy {
    type Target = RiskPolicy;

    fn deref(&self) -> &Self::Target {
        &self.policy
    }
}

/// Point-in-time account and market state used for one evaluation.
#[derive(Clone, Debug)]
pub struct RiskContext {
    pub account_id: AccountId,
    pub strategy_id: StrategyId,
    /// Execution venue the context was prepared for. It must match the
    /// intent's venue: eligibility is resolved against that venue's listing.
    pub venue: Venue,
    pub instrument: Option<Instrument>,
    pub quote: Option<QuoteSnapshot>,
    pub feed_health: FeedHealth,
    pub trade_date: TradeDate,
    pub now: LogicalNanos,
    /// Signed position in lots; negative is short.
    pub position_quantity: i64,
    pub pending_buy_quantity: Quantity,
    pub pending_sell_quantity: Quantity,
    pub gross_exposure_paise: Paise,
    pub pending_gross_exposure_paise: Paise,
    pub daily_pnl: Paise,
    pub open_orders: u32,
    pub orders_last_second: u32,
    pub session_open: bool,
    pub account_snapshot_available: bool,
    pub kill_switch_engaged: bool,
    pub reconciliation_blocked: bool,
    pub duplicate_intent: bool,
}

/// Evaluate every check, including checks after the first failure, so the
/// caller can report the complete reason for a rejection.
///
/// The policy must be a validated value; the context is validated at
/// the boundary (negative exposure values deny), never trusted blindly:
/// every caller-supplied financial state is treated as a claim, not a
/// fact. Overflow flows to `None` and fails rather than producing an
/// accepted decision.
pub fn evaluate_risk(
    intent: &OrderIntent,
    policy: &ValidatedRiskPolicy,
    context: &RiskContext,
) -> RiskAssessment {
    let instrument = context.instrument.as_ref();
    let quote = context.quote.as_ref();
    // A quote is evidence only for the venue it was observed on: the same
    // instrument can trade on NSE and BSE, where lot, tick, settlement and
    // price differ. Feed health and the collar must both use the intent's
    // venue, never "a quote for this instrument".
    let venue_quote = quote.filter(|value| {
        value.instrument_id == intent.instrument_id
            && value.venue == intent.venue
            && context.venue == intent.venue
    });
    // Context boundary: gross-exposure claims must be valid before any
    // projection arithmetic uses them (a negative current exposure would
    // understate the projected total and pass the limit dishonestly).
    let exposures_valid =
        context.gross_exposure_paise.0 >= 0 && context.pending_gross_exposure_paise.0 >= 0;
    let projected_exposure = notional(intent)
        .and_then(|order_value| order_value.checked_add(i128::from(context.gross_exposure_paise.0)))
        .and_then(|value| value.checked_add(i128::from(context.pending_gross_exposure_paise.0)));
    let checks = vec![
        check(
            RiskCheck::AccountAllowlist,
            policy.allowed_accounts.contains(&context.account_id)
                && intent.account_id == context.account_id,
            "account must match the active account and its allowlist",
        ),
        check(
            RiskCheck::StrategyAllowlist,
            policy.allowed_strategies.contains(&intent.strategy_id)
                && intent.strategy_id == context.strategy_id,
            "strategy must match an approved strategy identity",
        ),
        check(
            RiskCheck::AlgoAllowlist,
            policy.allowed_algos.contains(&intent.algo_id)
                && !intent.strategy_version.trim().is_empty()
                && !intent.reason.trim().is_empty(),
            "algo identity must be approved and the intent must carry a version and reason",
        ),
        check(
            RiskCheck::EvidenceReferences,
            !intent.evidence.is_empty(),
            "order intent must cite at least one recorded input event",
        ),
        check(
            RiskCheck::InstrumentAllowlist,
            policy.allowed_instruments.contains(&intent.instrument_id),
            "instrument must belong to the configured universe",
        ),
        check(
            RiskCheck::InstrumentReferenceData,
            instrument.is_some_and(|value| {
                value.instrument_id == intent.instrument_id
                    && context.venue == intent.venue
                    && value
                        .effective_listing_on(intent.venue, context.trade_date)
                        .is_some()
            }),
            "effective-dated instrument metadata must exist and be active on the intent's venue",
        ),
        check(
            RiskCheck::ProductEligibility,
            instrument.is_some_and(|value| {
                context.venue == intent.venue
                    && product_eligible(value, intent.venue, context.trade_date, intent.product)
            }),
            "trade-for-trade and Z-category listings are delivery-only; intraday is not eligible",
        ),
        check(
            RiskCheck::SessionOpen,
            context.session_open,
            "trading session must be open",
        ),
        check(
            RiskCheck::MarketFeedHealth,
            matches!(context.feed_health, FeedHealth::Healthy { age_ns } if age_ns <= policy.max_quote_age_ns)
                && venue_quote.is_some(),
            "quote must be two-sided, current, continuous and observed on the intent's venue",
        ),
        check(
            RiskCheck::AccountSnapshotAvailable,
            context.account_snapshot_available,
            "positions and account state must be available",
        ),
        check(
            RiskCheck::QuantityLimit,
            !intent.quantity.is_zero() && intent.quantity.get() <= policy.max_order_quantity.get(),
            "order quantity must fit the configured range",
        ),
        check(
            RiskCheck::LotAlignment,
            instrument
                .and_then(|value| value.effective_terms_on(intent.venue, context.trade_date))
                .is_some_and(|terms| {
                    terms.market_lot > 0 && intent.quantity.get() % terms.market_lot == 0
                }),
            "order quantity must align to the intended venue's effective lot size",
        ),
        check(
            RiskCheck::TickAlignment,
            instrument
                .and_then(|value| value.effective_terms_on(intent.venue, context.trade_date))
                .is_some_and(|terms| {
                    intent.limit_price.get().0 % terms.tick_assignment.tick_paise == 0
                }),
            "limit price must align to the intended venue's effective tick size",
        ),
        check(
            RiskCheck::OrderNotional,
            notional(intent)
                .is_some_and(|value| value <= i128::from(policy.max_order_notional_paise.0)),
            "limit-price notional must not exceed the configured order limit",
        ),
        check(
            RiskCheck::PriceCollar,
            price_within_collar(intent, venue_quote, policy.max_price_deviation_ppm),
            "limit price must remain inside the collar of a quote from the intent's venue",
        ),
        check(
            RiskCheck::PositionLimit,
            position_allowed(
                intent,
                context.position_quantity,
                context.pending_buy_quantity.get(),
                context.pending_sell_quantity.get(),
                policy.max_position_quantity.get(),
                policy.allow_short,
            ),
            "projected net position must fit the position policy",
        ),
        check(
            RiskCheck::GrossExposure,
            exposures_valid
                && projected_exposure
                    .is_some_and(|value| value <= i128::from(policy.max_gross_exposure_paise.0)),
            if exposures_valid {
                "current gross exposure plus order notional must fit the exposure policy"
            } else {
                "gross exposure values must be non-negative"
            },
        ),
        check(
            RiskCheck::DailyLossLimit,
            i128::from(context.daily_pnl.0) > -i128::from(policy.max_daily_loss_paise.0),
            "daily loss must remain inside the configured limit",
        ),
        check(
            RiskCheck::OpenOrderLimit,
            context.open_orders < policy.max_open_orders,
            "open-order count must remain below the configured limit",
        ),
        check(
            RiskCheck::OrderRateLimit,
            context.orders_last_second < policy.max_orders_per_second,
            "order rate must remain below the configured account limit",
        ),
        check(
            RiskCheck::KillSwitch,
            !context.kill_switch_engaged,
            "the kill switch must be released before exposure can increase",
        ),
        check(
            RiskCheck::Reconciliation,
            !context.reconciliation_blocked,
            "reconciliation must be complete and matched",
        ),
        check(
            RiskCheck::Idempotency,
            !context.duplicate_intent,
            "intent identity must not conflict with a prior request",
        ),
        check(
            RiskCheck::IntentExpiry,
            context.now < intent.expires_at,
            "intent must remain valid at the decision time",
        ),
    ];
    let accepted = checks.iter().all(|result| result.passed);
    RiskAssessment {
        intent_id: intent.intent_id,
        policy_version: policy.version.clone(),
        accepted,
        checks,
    }
}

/// Product eligibility from the listing state in force on `on_date`.
///
/// Tradability and product eligibility are different questions: a
/// trade-for-trade or Z-category security is `Active` and can be delivered,
/// but its settlement regime (no netting) makes intraday ineligible. An
/// absent or inactive listing is not eligible for anything — callers must
/// fail closed rather than infer from a symbol.
fn product_eligible(
    instrument: &trading_core::Instrument,
    venue: trading_core::Venue,
    on_date: trading_core::TradeDate,
    product: trading_core::Product,
) -> bool {
    let Some(listing) = instrument.effective_listing_on(venue, on_date) else {
        return false;
    };
    let intraday = product == trading_core::Product::Intraday;
    if intraday && listing.z_category {
        return false;
    }
    match listing.settlement_regime {
        trading_core::SettlementRegime::Standard => true,
        trading_core::SettlementRegime::TradeForTrade => !intraday,
    }
}

fn check(check: RiskCheck, passed: bool, detail: &str) -> RiskCheckResult {
    RiskCheckResult {
        check,
        passed,
        detail: detail.to_owned(),
    }
}

fn notional(intent: &OrderIntent) -> Option<i128> {
    i128::from(intent.limit_price.get().0).checked_mul(i128::from(intent.quantity.get()))
}

fn price_within_collar(
    intent: &OrderIntent,
    quote: Option<&QuoteSnapshot>,
    maximum_deviation_ppm: u32,
) -> bool {
    // Exact: with M₂ = bid + ask and order price p (exact integers), the
    // deviation test |2p − M₂| · 10⁶ ≤ M₂ · deviation never rounds the
    // reference price first, so a limit exactly at the half-paise midpoint
    // is evaluated against the true midpoint.
    let Some(midpoint) = quote.and_then(QuoteSnapshot::midpoint_doubled) else {
        return false;
    };
    let doubled_limit = i128::from(intent.limit_price.get().0) * 2;
    let doubled_delta = (doubled_limit - midpoint.value()).unsigned_abs();
    let lhs = doubled_delta * 1_000_000_u128;
    // Reference magnitude, matching the pre-change formula: the collar
    // bound is the absolute reference times the deviation tolerance.
    let rhs = midpoint.value().unsigned_abs() * u128::from(maximum_deviation_ppm);
    lhs <= rhs
}

fn position_allowed(
    intent: &OrderIntent,
    current: i64,
    pending_buys: i64,
    pending_sells: i64,
    maximum: i64,
    allow_short: bool,
) -> bool {
    let current = i128::from(current);
    let pending_buys = i128::from(pending_buys);
    let pending_sells = i128::from(pending_sells);
    let new_quantity = i128::from(intent.quantity.get());
    let upper = current
        + pending_buys
        + if intent.side == Side::Buy {
            new_quantity
        } else {
            0
        };
    let lower = current
        - pending_sells
        - if intent.side == Side::Sell {
            new_quantity
        } else {
            0
        };
    let maximum = u128::from(maximum.unsigned_abs());
    (allow_short || lower >= 0)
        && upper.unsigned_abs() <= maximum
        && lower.unsigned_abs() <= maximum
}
