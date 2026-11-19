#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Active order-book conformance driver.
//!
//! Reads the same line-oriented corpus as the C++ oracle driver and writes one
//! canonical JSON snapshot per `snapshot` operation. This binary is a
//! verification target; it is not linked into any runtime path.

use std::io::{
    BufRead, //
    Write,
};

use market_book::OrderBook;
use market_types::{
    Side, //
    model::{Level, Price, Quantity, QuantityChange, QuantityDelta},
};

fn print_levels(levels: &[Level], out: &mut impl Write) -> std::io::Result<()> {
    let mut first = true;
    for level in levels {
        if !first {
            write!(out, ",")?;
        }
        first = false;
        write!(
            out,
            "{{\"price\":{},\"quantity\":{}}}",
            level.price.raw(),
            level.quantity.raw()
        )?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let mut book = OrderBook::new();
    for line in stdin.lock().lines() {
        let line = line?;
        let mut parts = line.split_whitespace();
        let Some(op) = parts.next() else {
            continue;
        };
        match op {
            "abs" | "delta" => {
                let side = match parts.next() {
                    Some("buy") => Side::Buy,
                    Some("sell") => Side::Sell,
                    other => return Err(format!("unknown side {other:?}").into()),
                };
                let price: i64 = parts.next().unwrap_or("0").parse()?;
                let value: i64 = parts.next().unwrap_or("0").parse()?;
                let change = if op == "abs" {
                    let quantity = Quantity::new(value).map_err(|error| {
                        std::io::Error::new(std::io::ErrorKind::InvalidInput, error.to_string())
                    })?;
                    QuantityChange::Absolute(quantity)
                } else {
                    QuantityChange::Delta(QuantityDelta::from_raw(value))
                };
                book.apply(side, Price::from_raw(price), change)?;
            }
            "snapshot" => {
                let snapshot = book.to_snapshot();
                write!(out, "{{\"bids\":[")?;
                print_levels(&snapshot.bids, &mut out)?;
                write!(out, "],\"asks\":[")?;
                print_levels(&snapshot.asks, &mut out)?;
                writeln!(out, "]}}")?;
            }
            other => return Err(format!("unknown op: {other}").into()),
        }
    }
    out.flush()?;
    Ok(())
}
