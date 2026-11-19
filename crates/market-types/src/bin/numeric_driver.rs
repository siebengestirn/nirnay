#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Active fixed-point conformance driver.
//!
//! Reads the same versioned numeric corpus as the C++ oracle driver and writes
//! one canonical result line per operation. Verification target only.

use std::io::{
    BufRead, //
    Write,
};

use market_types::fixed::{
    format_scaled, //
    parse_scaled,
    rescale_exact,
    scale_factor,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line?;
        let mut parts = line.split_whitespace();
        let Some(op) = parts.next() else {
            continue;
        };
        match op {
            "parse" => {
                let text = parts.next().unwrap_or("");
                let scale: u16 = parts.next().unwrap_or("0").parse()?;
                match parse_scaled(text, scale as u8) {
                    Ok(value) => writeln!(out, "parse {value}")?,
                    Err(_) => writeln!(out, "error parse")?,
                }
            }
            "format" => {
                let raw: i64 = parts.next().unwrap_or("0").parse()?;
                let scale: u16 = parts.next().unwrap_or("0").parse()?;
                if scale_factor(scale as u8).is_ok() {
                    writeln!(out, "format {}", format_scaled(raw, scale as u8))?;
                } else {
                    writeln!(out, "error format")?;
                }
            }
            "rescale" => {
                let raw: i64 = parts.next().unwrap_or("0").parse()?;
                let from: u16 = parts.next().unwrap_or("0").parse()?;
                let to: u16 = parts.next().unwrap_or("0").parse()?;
                match rescale_exact(raw, from as u8, to as u8) {
                    Ok(value) => writeln!(out, "rescale {value}")?,
                    Err(_) => writeln!(out, "error rescale")?,
                }
            }
            other => return Err(format!("unknown op: {other}").into()),
        }
    }
    out.flush()?;
    Ok(())
}
