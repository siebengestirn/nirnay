#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fundamental_domain::{
    AccountingBasis, //
    CompanyId,
    CurrencyCode,
    EvidenceLocator,
    ExactDecimal,
    ExpandedName,
    FactOrigin,
    FinancialConcept,
    FinancialFact,
    FinancialUnit,
    Magnitude,
    ReportingPeriod,
    StatementScope,
    validate_fact_set,
}; //
use jiff::Timestamp;
use jiff::civil::Date;

fn timestamp(second: i64) -> Timestamp {
    Timestamp::new(second, 0).expect("timestamp")
}

fn reported(id: &str, value: &str) -> FinancialFact {
    let mut fact = FinancialFact {
        schema_version: 1,
        fact_id: id.to_owned(),
        fact_key: String::new(),
        revision: 1,
        supersedes: None,
        company_id: CompanyId::new("co-example").expect("company"),
        concept: FinancialConcept {
            name: ExpandedName {
                namespace_uri: "https://xbrl.example/ind-as".to_owned(),
                local_name: "RevenueFromOperations".to_owned(),
            },
        },
        period: ReportingPeriod::Duration {
            start: Date::new(2025, 4, 1).expect("start"),
            end: Date::new(2026, 3, 31).expect("end"),
        },
        statement_scope: StatementScope::Standalone,
        accounting_basis: AccountingBasis::IndAs,
        dimensions: Vec::new(),
        unit: FinancialUnit::Currency {
            currency: CurrencyCode::new("INR").expect("currency"),
            magnitude: Magnitude::Crore,
        },
        value: ExactDecimal::new(value, 2).expect("value"),
        published_at: Some(timestamp(10)),
        observed_at: timestamp(11),
        ingested_at: timestamp(12),
        origin: FactOrigin::Reported {
            source_id: "mca21.public_documents".to_owned(),
            filing_id: "filing-aoc4-2026".to_owned(),
            document_sha256: "b".repeat(64),
            locator: EvidenceLocator::Xbrl {
                fact_id: "f-001".to_owned(),
                context_ref: "ctx-2026".to_owned(),
                unit_ref: "INR_Crore".to_owned(),
            },
        },
    };
    fact.refresh_fact_key().expect("fact key");
    fact
}

#[test]
fn restatement_is_a_new_immutable_revision() {
    let first = reported("fact-revenue-v1", "10000");
    let mut second = first.clone();
    second.fact_id = "fact-revenue-v2".to_owned();
    second.revision = 2;
    second.supersedes = Some(first.fact_id.clone());
    second.value = ExactDecimal::new("9500", 2).expect("new value");
    second.published_at = Some(timestamp(20));
    second.observed_at = timestamp(21);
    second.ingested_at = timestamp(22);
    assert_eq!(
        first.fact_key, second.fact_key,
        "a value restatement must retain the logical fact key",
    );
    validate_fact_set(&[second, first]).expect("complete revision chain");
}

#[test]
fn rejects_restatement_that_rewrites_fact_identity() {
    let first = reported("fact-revenue-v1", "10000");
    let mut second = first.clone();
    second.fact_id = "fact-revenue-v2".to_owned();
    second.revision = 2;
    second.supersedes = Some(first.fact_id.clone());
    second.period = ReportingPeriod::Duration {
        start: Date::new(2024, 4, 1).expect("start"),
        end: Date::new(2025, 3, 31).expect("end"),
    };
    second.refresh_fact_key().expect("new fact identity");
    assert!(
        validate_fact_set(&[first, second]).is_err(),
        "a restatement cannot change the logical fact identity",
    );
}
