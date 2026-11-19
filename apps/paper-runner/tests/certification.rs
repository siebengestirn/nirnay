#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.8 end-to-end paper certification tests. Synthetic session, simulated
//! broker, no live data or endpoint.

use std::{
    //
    collections::BTreeSet,
    path::PathBuf,
    process::Command,
};

use paper_runner::{
    PaperInput, //
    run_paper,
};
use risk_engine::RiskPolicy;
use trading_core::{
    AccountId, //
    AlgoId,
    InstrumentId, //
    Paise,
    Quantity, //
    StrategyId,
    TradeDate,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn input(behavior: &str) -> Result<PaperInput, Box<dyn std::error::Error>> {
    let mid = [10, 9, 8, 7, 8, 9, 10, 9, 8, 7];
    let quotes = mid
        .iter()
        .enumerate()
        .map(|(index, value)| (*value - 1, *value + 1, (index as u64) + 1))
        .collect();
    let id = InstrumentId::new("nse.cash.paperidx")?;
    let policy = RiskPolicy {
        version: "paper-risk-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(1)?]),
        allowed_strategies: BTreeSet::from([StrategyId::new(2)?]),
        allowed_algos: BTreeSet::from([AlgoId::new(3)?]),
        allowed_instruments: BTreeSet::from([id]),
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(100_000),
        max_position_quantity: Quantity::new(100)?,
        max_gross_exposure_paise: Paise(100_000),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 10,
        max_orders_per_second: 10,
        max_price_deviation_ppm: 200_000,
        max_quote_age_ns: 1_500_000_000,
        allow_short: false,
    };
    Ok(PaperInput {
        schema_version: 1,
        instrument_id: "nse.cash.paperidx".to_owned(),
        account_id: 1,
        strategy_id: 2,
        algo_id: 3,
        trade_date: TradeDate::new(2026, 10, 28)?,
        policy,
        fast: 2,
        slow: 3,
        quantity: 2,
        min_observations: 3,
        aggressive_limit_bps: 0,
        quotes,
        feed_age_ns: 100,
        disk_available_bytes: 10_000_000,
        broker_behavior: behavior.to_owned(),
    })
}

#[test]
fn end_to_end_paper_path_produces_bound_decisions() -> TestResult {
    let directory = tempfile::tempdir()?;
    let run = run_paper(&input("fill")?, &directory.path().join("oms.journal"))?;
    let report = run.report;
    assert_eq!(report.schema_version, 1, "the report has a known schema");
    assert_eq!(
        report.events_seen, 10,
        "all synthetic events were projected"
    );
    assert!(
        report.intents_emitted > 0,
        "the strategy emitted at least one intent"
    );
    assert_eq!(
        report.reservations, report.intents_emitted,
        "every intent was risk-reserved when healthy"
    );
    assert!(
        report.health_safe_for_new_risk,
        "the healthy session permits new risk"
    );
    assert!(
        !report.run_digest.is_empty(),
        "the report has a content digest"
    );
    for decision in &report.decisions {
        assert!(decision.risk_accepted, "the decision was accepted by risk");
        assert!(decision.permit_id.is_some(), "a permit id is bound");
        assert!(
            decision.broker_order_id.is_some(),
            "a broker identity is bound"
        );
        assert_eq!(
            decision.oms_final_state, "Filled",
            "the simulated broker filled the order"
        );
    }
    Ok(())
}

#[test]
fn fresh_process_replay_is_byte_identical() -> TestResult {
    let directory = tempfile::tempdir()?;
    let input_path = directory.path().join("input.json");
    std::fs::write(&input_path, serde_json::to_vec(&input("fill")?)?)?;
    let binary = env!("CARGO_BIN_EXE_paper_run");
    let mut reports = Vec::new();
    for name in ["first.json", "second.json"] {
        let output = directory.path().join(name);
        let oms = directory.path().join(name.replace(".json", "-oms"));
        let status = Command::new(binary)
            .args([
                input_path.to_string_lossy().as_ref(),
                output.to_string_lossy().as_ref(),
                oms.to_string_lossy().as_ref(),
            ])
            .output()?;
        let detail = String::from_utf8_lossy(&status.stderr);
        assert!(status.status.success(), "paper runner completed: {detail}");
        reports.push(std::fs::read(output)?);
    }
    assert_eq!(
        reports[0], reports[1],
        "fresh-process reports are byte-identical"
    );
    Ok(())
}

#[test]
fn stale_feed_blocks_all_new_exposure() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut source = input("fill")?;
    source.feed_age_ns = 5_000_000_000;
    let run = run_paper(&source, &directory.path().join("oms.journal"))?;
    assert!(
        !run.report.health_safe_for_new_risk,
        "a stale feed makes the runtime unsafe"
    );
    assert_eq!(run.report.reservations, 0, "no exposure is reserved");
    assert!(
        run.report
            .decisions
            .iter()
            .all(|decision| !decision.risk_accepted),
        "no decision was risk-accepted"
    );
    assert!(
        run.report
            .decisions
            .iter()
            .all(|decision| decision.oms_final_state == "not_created"),
        "no OMS order was created"
    );
    Ok(())
}

#[test]
fn risk_rejection_creates_no_order() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut source = input("fill")?;
    source.policy.allowed_accounts = BTreeSet::from([AccountId::new(999)?]);
    let run = run_paper(&source, &directory.path().join("oms.journal"))?;
    assert!(
        run.report.intents_emitted > 0,
        "the strategy still emits intents"
    );
    assert_eq!(run.report.reservations, 0, "risk refused every intent");
    assert!(
        run.report
            .decisions
            .iter()
            .all(|decision| decision.oms_final_state == "not_created"),
        "a rejected intent creates no OMS order"
    );
    Ok(())
}

#[test]
fn timeout_leaves_an_unresolved_oms_order() -> TestResult {
    let directory = tempfile::tempdir()?;
    let run = run_paper(&input("timeout")?, &directory.path().join("oms.journal"))?;
    assert_eq!(
        run.report.reservations, run.report.intents_emitted,
        "risk accepted the intents"
    );
    assert!(
        run.report
            .decisions
            .iter()
            .all(|decision| decision.oms_final_state == "Unknown"),
        "a dispatch timeout leaves every order UNKNOWN"
    );
    assert!(
        run.report
            .decisions
            .iter()
            .all(|decision| decision.broker_order_id.is_none()),
        "no broker identity was confirmed"
    );
    assert!(
        !run.report.health_safe_for_new_risk,
        "the final report reflects leftover UNKNOWN orders"
    );
    Ok(())
}

#[test]
fn report_digest_binds_the_inputs() -> TestResult {
    let directory = tempfile::tempdir()?;
    let base = input("fill")?;
    let mut changed = base.clone();
    changed.quantity = 3;
    let first = run_paper(&base, &directory.path().join("a.journal"))?
        .report
        .run_digest;
    let second = run_paper(&changed, &directory.path().join("b.journal"))?
        .report
        .run_digest;
    assert_ne!(
        first, second,
        "different inputs produce different report digests"
    );

    let mut policy_changed = input("fill")?;
    policy_changed.policy.version = "paper-risk-v2".to_owned();
    let third = run_paper(&policy_changed, &directory.path().join("c.journal"))?.report;
    assert_ne!(
        first, third.run_digest,
        "a policy version change moves the digest"
    );
    assert_eq!(
        third.input_digest.len(),
        64,
        "the input digest is a SHA-256 value"
    );
    Ok(())
}

#[test]
fn no_live_or_broker_sdk_dependency() -> TestResult {
    let manifest = include_str!("../Cargo.toml").to_ascii_lowercase();
    for forbidden in ["reqwest", "tokio", "upstox-feed-proto", "fyers", "kite"] {
        assert!(
            !manifest.contains(forbidden),
            "manifest must not depend on {forbidden}"
        );
    }
    Ok(())
}

#[test]
fn public_api_has_no_submission_path() -> TestResult {
    let source =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))?;
    for forbidden in [
        "submit_order",
        "place_order",
        "live_trade",
        "broker_api_key",
    ] {
        assert!(
            !source.contains(forbidden),
            "the harness exposes no {forbidden} path"
        );
    }
    Ok(())
}
