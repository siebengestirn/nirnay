//! Rust side of the conformance harness. Reads the same line-oriented
//! operations as `cpp/tests/conform_driver.cpp`, applies them to the Rust
//! `OrderBook`, and prints the same JSON shape so the two planes can be
//! diffed byte-for-byte. Not part of the production binary.

use std::io::{self, BufRead, BufWriter, Write};

use nirnay_extras::{
    Side,
    model::{Level, Price, Quantity, QuantityChange, QuantityDelta},
    orderbook::OrderBook,
};

fn level_json(level: &Level) -> String {
    format!(
        "{{\"price\":{},\"quantity\":{}}}",
        level.price.raw(),
        level.quantity.raw()
    )
}

fn main() -> anyhow::Result<()> {
    let mut out = BufWriter::new(io::stdout());
    let mut book = OrderBook::new();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let mut parts = line.split_whitespace();
        let Some(op) = parts.next() else { continue };
        match op {
            "abs" | "delta" => {
                let side = match parts.next() {
                    Some("buy") => Side::Buy,
                    Some("sell") => Side::Sell,
                    other => anyhow::bail!("unknown side {other:?}"),
                };
                let price: i64 = parts.next().unwrap_or("0").parse()?;
                let value: i64 = parts.next().unwrap_or("0").parse()?;
                let change = if op == "abs" {
                    QuantityChange::Absolute(Quantity::new(value)?)
                } else {
                    QuantityChange::Delta(QuantityDelta::from_raw(value))
                };
                book.apply(side, Price::from_raw(price), change)?;
            }
            "snapshot" => {
                let snapshot = book.to_snapshot();
                let bids = snapshot
                    .bids
                    .iter()
                    .map(level_json)
                    .collect::<Vec<_>>()
                    .join(",");
                let asks = snapshot
                    .asks
                    .iter()
                    .map(level_json)
                    .collect::<Vec<_>>()
                    .join(",");
                writeln!(out, "{{\"bids\":[{bids}],\"asks\":[{asks}]}}")?;
            }
            other => anyhow::bail!("unknown op {other}"),
        }
    }
    Ok(())
}
