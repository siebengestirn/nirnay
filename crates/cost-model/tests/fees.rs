//! Fee-schedule selection and integer-rounding contract tests.

use cost_model::{ChargeBook, ChargeRate, FeeError};
use trading_core::{DomainError, Paise, Product, Side, TradeDate};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn date(year: i32, month: u32, day: u32) -> Result<TradeDate, DomainError> {
    TradeDate::new(year as i16, month as i8, day as i8)
        .ok()
        .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))
}

fn rate(effective_from: TradeDate, brokerage_ppb: u64) -> ChargeRate {
    ChargeRate {
        effective_from,
        product: Product::Intraday,
        side: Side::Buy,
        brokerage_ppb,
        stt_ppb: 0,
        exchange_ppb: 0,
        sebi_ppb: 0,
        stamp_ppb: 0,
        gst_ppb: 0,
    }
}

#[test]
fn latest_effective_schedule_is_selected() -> TestResult {
    let book = ChargeBook::new(vec![
        rate(date(2026, 1, 1)?, 100_000_000),
        rate(date(2026, 10, 1)?, 50_000_000),
    ])?;
    assert_eq!(book.schedule_count(), 2);

    // 5% of 1,000 paise is 50 paise; the January 10% schedule is not used.
    let charges = book.calculate(
        date(2026, 10, 28)?,
        Product::Intraday,
        Side::Buy,
        Paise(1_000),
    )?;
    assert_eq!(charges.brokerage, Paise(50));
    Ok(())
}

#[test]
fn levy_rounds_half_up_to_the_nearest_paise() -> TestResult {
    // 10% of turnover, expressed in parts per billion.
    let book = ChargeBook::new(vec![rate(date(2026, 1, 1)?, 100_000_000)])?;
    let at = date(2026, 10, 28)?;

    assert_eq!(
        book.calculate(at, Product::Intraday, Side::Buy, Paise(4))?
            .brokerage,
        Paise(0)
    );
    assert_eq!(
        book.calculate(at, Product::Intraday, Side::Buy, Paise(5))?
            .brokerage,
        Paise(1)
    );
    assert_eq!(
        book.calculate(at, Product::Intraday, Side::Buy, Paise(15))?
            .brokerage,
        Paise(2)
    );
    Ok(())
}

#[test]
fn gst_is_charged_on_brokerage_exchange_and_sebi_only() -> TestResult {
    let book = ChargeBook::new(vec![ChargeRate {
        effective_from: date(2026, 1, 1)?,
        product: Product::Intraday,
        side: Side::Buy,
        brokerage_ppb: 100_000_000, // 10% -> 100 paise on 1,000
        stt_ppb: 0,
        exchange_ppb: 50_000_000, // 5% -> 50 paise
        sebi_ppb: 10_000_000,     // 1% -> 10 paise
        stamp_ppb: 0,
        gst_ppb: 100_000_000, // 10% of the 160 paise taxable base
    }])?;
    let charges = book.calculate(
        date(2026, 10, 28)?,
        Product::Intraday,
        Side::Buy,
        Paise(1_000),
    )?;
    assert_eq!(charges.brokerage, Paise(100));
    assert_eq!(charges.exchange_transaction, Paise(50));
    assert_eq!(charges.sebi_turnover, Paise(10));
    assert_eq!(charges.gst, Paise(16));
    assert_eq!(charges.total()?, Paise(176));
    Ok(())
}

#[test]
fn duplicate_effective_dates_are_rejected() -> TestResult {
    let duplicate = rate(date(2026, 1, 1)?, 1);
    assert!(matches!(
        ChargeBook::new(vec![duplicate.clone(), duplicate]),
        Err(FeeError::DuplicateEffectiveDate)
    ));
    Ok(())
}

#[test]
fn missing_schedule_fails_closed() -> TestResult {
    let book = ChargeBook::new(vec![rate(date(2027, 1, 1)?, 1)])?;
    assert!(matches!(
        book.calculate(
            date(2026, 10, 28)?,
            Product::Intraday,
            Side::Buy,
            Paise(1_000),
        ),
        Err(FeeError::MissingSchedule)
    ));
    Ok(())
}

#[test]
fn out_of_range_rate_is_rejected() -> TestResult {
    assert!(matches!(
        ChargeBook::new(vec![rate(date(2026, 1, 1)?, 1_000_000_001)]),
        Err(FeeError::InvalidRate)
    ));
    Ok(())
}

#[test]
fn charge_book_deserializes_through_validation() -> TestResult {
    let encoded = serde_json::to_string(&vec![rate(date(2026, 1, 1)?, 1)])?;
    let book: ChargeBook = serde_json::from_str(&encoded)?;
    assert_eq!(book.schedule_count(), 1);

    let duplicate = rate(date(2026, 1, 1)?, 1);
    let encoded = serde_json::to_string(&vec![duplicate.clone(), duplicate])?;
    assert!(serde_json::from_str::<ChargeBook>(&encoded).is_err());
    Ok(())
}
