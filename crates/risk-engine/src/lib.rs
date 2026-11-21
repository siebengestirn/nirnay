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

use std::collections::{
    BTreeMap, //
    BTreeSet,
};

use serde::{
    Deserialize, //
    Serialize,
};
use trading_core::{
    AccountId, //
    AlgoId,
    BorrowAvailability,
    DomainError,
    FeedHealth,
    Greeks,
    Instrument,
    InstrumentId,
    LiquidityDepth,
    LogicalNanos,
    OrderIntent,
    Paise,
    Quantity,
    QuoteSnapshot,
    RiskAssessment,
    RiskCheck,
    RiskCheckResult,
    Side,
    StrategyId,
    TradeDate,
    Venue,
};

/// Deterministic, uncalibrated stress scenario. A negative `price_shift_ppm`
/// is a downward parallel shift. Every term is an exact integer ratio in ppm;
/// no floating point is used in projected-loss arithmetic.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StressScenario {
    pub price_shift_ppm: i64,
    pub liquidity_haircut_ppm: u32,
    pub correlation_multiplier_ppm: u32,
}

/// Typed placeholder limits for synthetic per-instrument Greeks. Sensitivities
/// use the same 10^6 fixed-point scaling as [`Greeks`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GreekLimits {
    pub max_abs_delta_notional_paise: i64,
    pub max_abs_gamma: i64,
    pub max_abs_vega: i64,
    pub max_abs_theta: i64,
}

/// Effective risk policy. All limits are mandatory; validation rejects an
/// empty allowlist, a zero limit or an impossible collar. The sector, stress,
/// liquidity, Greeks and borrow controls are opt-in: `None` means the control
/// has no configured cap or reference data, and the corresponding check is
/// inert. Once configured, each control fails closed on missing evidence.
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
    pub max_strategy_exposure_paise: Paise,
    pub max_sector_exposure_paise: Option<Paise>,
    pub stress_scenario: Option<StressScenario>,
    pub max_stress_loss_paise: Option<Paise>,
    pub max_participation_ppm: Option<u32>,
    pub greek_limits: Option<GreekLimits>,
    pub max_borrow_quantity: Option<Quantity>,
    pub max_financing_cost_paise: Option<Paise>,
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
            || self.max_strategy_exposure_paise.0 <= 0
        {
            return Err(DomainError::InvalidInstrument(
                "risk policy has invalid or empty limits".to_owned(),
            ));
        }
        if self.max_sector_exposure_paise.is_some_and(|cap| cap.0 <= 0)
            || self.max_stress_loss_paise.is_some_and(|cap| cap.0 <= 0)
            || self
                .max_participation_ppm
                .is_some_and(|ppm| ppm == 0 || ppm > 1_000_000)
            || self
                .max_borrow_quantity
                .is_some_and(|quantity| quantity.is_zero())
            || self.max_financing_cost_paise.is_some_and(|cost| cost.0 < 0)
        {
            return Err(DomainError::InvalidInstrument(
                "risk policy has an invalid optional limit".to_owned(),
            ));
        }
        // A scenario without a loss cap (or a cap without a scenario) would
        // silently disable the stress check: both must be configured together.
        if self.stress_scenario.is_some() != self.max_stress_loss_paise.is_some() {
            return Err(DomainError::InvalidInstrument(
                "stress scenario and stress loss cap must be configured together".to_owned(),
            ));
        }
        if let Some(scenario) = self.stress_scenario
            && (scenario.price_shift_ppm.unsigned_abs() > 1_000_000
                || scenario.liquidity_haircut_ppm > 1_000_000
                || scenario.correlation_multiplier_ppm == 0
                || scenario.correlation_multiplier_ppm > 10_000_000)
        {
            return Err(DomainError::InvalidInstrument(
                "stress scenario ratios are outside the supported range".to_owned(),
            ));
        }
        if let Some(limits) = self.greek_limits
            && (limits.max_abs_delta_notional_paise < 0
                || limits.max_abs_gamma < 0
                || limits.max_abs_vega < 0
                || limits.max_abs_theta < 0)
        {
            return Err(DomainError::InvalidInstrument(
                "Greek limits cannot be negative".to_owned(),
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
    /// Current (pre-order) strategy exposure; the projection adds this order.
    pub strategy_exposure_paise: Paise,
    /// Synthetic sector label resolved by the authority; `None` when the
    /// instrument has no sector mapping.
    pub sector_label: Option<String>,
    /// Current (pre-order) exposure in the resolved sector; the projection
    /// adds this order.
    pub sector_exposure_paise: Paise,
    /// Signed lots held in this account, for deterministic stress projection.
    pub positions: BTreeMap<InstrumentId, i64>,
    /// Quote-derived exact marks by instrument; a position without a mark
    /// fails the stress check closed.
    pub marks_paise: BTreeMap<InstrumentId, Paise>,
    /// Synthetic displayed depth and ADV for the intent's instrument.
    pub depth: Option<LiquidityDepth>,
    /// Synthetic Greeks for the intent's instrument.
    pub greeks: Option<Greeks>,
    /// Whether the instrument is flagged as a derivative in synthetic
    /// reference metadata.
    pub is_derivative: bool,
    /// Synthetic short-borrow evidence for the intent's instrument.
    pub borrow: Option<BorrowAvailability>,
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
    let exposures_valid = context.gross_exposure_paise.0 >= 0
        && context.pending_gross_exposure_paise.0 >= 0
        && context.strategy_exposure_paise.0 >= 0
        && context.sector_exposure_paise.0 >= 0;
    let order_notional = notional(intent);
    let projected_exposure = order_notional
        .and_then(|order_value| order_value.checked_add(i128::from(context.gross_exposure_paise.0)))
        .and_then(|value| value.checked_add(i128::from(context.pending_gross_exposure_paise.0)));
    let projected_strategy = order_notional
        .and_then(|value| value.checked_add(i128::from(context.strategy_exposure_paise.0)));
    let projected_sector = order_notional
        .and_then(|value| value.checked_add(i128::from(context.sector_exposure_paise.0)));
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
            RiskCheck::StrategyExposure,
            exposures_valid
                && projected_strategy
                    .is_some_and(|value| value <= i128::from(policy.max_strategy_exposure_paise.0)),
            "projected strategy notional must fit the strategy cap",
        ),
        check(
            RiskCheck::SectorExposure,
            sector_allowed(policy, context, projected_sector),
            "a configured sector cap requires a label and a projected notional within it",
        ),
        check(
            RiskCheck::StressLoss,
            stress_allowed(policy, context, order_notional),
            "deterministic projected stress loss must fit the configured cap",
        ),
        check(
            RiskCheck::LiquidityParticipation,
            participation_allowed(policy, context, intent),
            "order notional must fit the configured fraction of synthetic displayed depth",
        ),
        check(
            RiskCheck::Greeks,
            greeks_allowed(policy, context),
            "a derivative-flagged instrument needs greeks within every configured limit",
        ),
        check(
            RiskCheck::BorrowFinancing,
            borrow_allowed(policy, context, intent),
            "a short needs synthetic borrow evidence within the configured cost cap",
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

/// Sector check. With no configured cap the control is inert; once configured,
/// an unlabelled instrument cannot pass by default.
fn sector_allowed(
    policy: &ValidatedRiskPolicy,
    context: &RiskContext,
    projected: Option<i128>,
) -> bool {
    let Some(cap) = policy.max_sector_exposure_paise else {
        return true;
    };
    context
        .sector_label
        .as_ref()
        .is_some_and(|label| !label.trim().is_empty())
        && projected.is_some_and(|value| value <= i128::from(cap.0))
}

/// Deterministic projected loss: parallel price shift over the exact notional
/// of every marked position plus this order, after a liquidity haircut and a
/// linear correlation multiplier. All ratios are integer ppm and overflow
/// denies. A position without a mark fails closed.
fn stress_allowed(
    policy: &ValidatedRiskPolicy,
    context: &RiskContext,
    order_notional: Option<i128>,
) -> bool {
    let (Some(scenario), Some(cap)) = (policy.stress_scenario, policy.max_stress_loss_paise) else {
        return true;
    };
    let Some(order) = order_notional.and_then(|value| u128::try_from(value.max(0)).ok()) else {
        return false;
    };
    let mut exposure = order;
    for (instrument_id, quantity) in &context.positions {
        let Some(mark) = context.marks_paise.get(instrument_id) else {
            return false;
        };
        let magnitude = u128::from(quantity.unsigned_abs());
        let Some(value) = magnitude.checked_mul(u128::from(mark.0.unsigned_abs())) else {
            return false;
        };
        let Some(next) = exposure.checked_add(value) else {
            return false;
        };
        exposure = next;
    }
    let haircut = 1_000_000_u128.saturating_sub(u128::from(scenario.liquidity_haircut_ppm));
    let loss = exposure
        .checked_mul(haircut)
        .and_then(|value| value.checked_div(1_000_000))
        .and_then(|value| value.checked_mul(u128::from(scenario.price_shift_ppm.unsigned_abs())))
        .and_then(|value| value.checked_div(1_000_000))
        .and_then(|value| value.checked_mul(u128::from(scenario.correlation_multiplier_ppm)))
        .and_then(|value| value.checked_div(1_000_000));
    loss.is_some_and(|value| i128::try_from(value).is_ok_and(|loss| loss <= i128::from(cap.0)))
}

/// Synthetic participation check. The reference quantity is the smaller of
/// displayed depth and ADV, valued at the intent's exact limit price; the
/// order notional may consume at most `max_participation_ppm` of it.
fn participation_allowed(
    policy: &ValidatedRiskPolicy,
    context: &RiskContext,
    intent: &OrderIntent,
) -> bool {
    let Some(max_ppm) = policy.max_participation_ppm else {
        return true;
    };
    let Some(depth) = context.depth else {
        return false;
    };
    let quantity = depth.displayed_quantity.get().min(depth.adv_quantity.get());
    let reference = i128::from(quantity).checked_mul(i128::from(intent.limit_price.get().0));
    let allowance = reference
        .and_then(|value| value.checked_mul(i128::from(max_ppm)))
        .and_then(|value| value.checked_div(1_000_000));
    notional(intent)
        .is_some_and(|order| order >= 0 && allowance.is_some_and(|limit| order <= limit))
}

/// Synthetic Greeks check. A derivative-flagged instrument without greeks is
/// denied even when no limits are configured; configured limits bound each
/// absolute sensitivity.
fn greeks_allowed(policy: &ValidatedRiskPolicy, context: &RiskContext) -> bool {
    if !context.is_derivative {
        return true;
    }
    let Some(greeks) = context.greeks else {
        return false;
    };
    let Some(limits) = policy.greek_limits else {
        return true;
    };
    let within = |observed: i64, limit: i64| {
        i128::from(observed.unsigned_abs()) <= i128::from(limit.unsigned_abs())
    };
    within(
        greeks.delta_notional_paise,
        limits.max_abs_delta_notional_paise,
    ) && within(greeks.gamma_scaled, limits.max_abs_gamma)
        && within(greeks.vega_scaled, limits.max_abs_vega)
        && within(greeks.theta_scaled, limits.max_abs_theta)
}

/// Synthetic borrow check. With no configured quantity cap the control is
/// inert; a configured short must carry available borrow within the cap and,
/// when a cost cap is configured, within that cost too.
fn borrow_allowed(
    policy: &ValidatedRiskPolicy,
    context: &RiskContext,
    intent: &OrderIntent,
) -> bool {
    if intent.side != Side::Sell {
        return true;
    }
    let Some(max_quantity) = policy.max_borrow_quantity else {
        return true;
    };
    let Some(borrow) = context.borrow else {
        return false;
    };
    let quantity_ok = borrow.available_quantity.get() >= intent.quantity.get()
        && borrow.available_quantity.get() <= max_quantity.get();
    let cost_ok = policy
        .max_financing_cost_paise
        .is_none_or(|cap| borrow.financing_cost_paise.0 <= cap.0);
    quantity_ok && cost_ok
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
