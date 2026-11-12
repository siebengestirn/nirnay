//! Rust SBE dump: one canonical line per message, matching
//! `cpp/tests/sbe_dump.cpp` byte-for-byte.

use std::io::Write;

use nirnay_extras::util::{
    AlignedSbeReader, OrderbookSchema,
    dtos::challenge_orderbook::{AnyMessage, PriceLevelUpdateDomain, SnapshotDomain},
};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("tests/golden/sample_replay.sbe");
    let chunk: usize = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(4096);
    let mut reader = AlignedSbeReader::<OrderbookSchema>::open_with_chunk_size(path, chunk)?;
    let mut out = std::io::BufWriter::new(std::io::stdout());
    while let Some(message) = reader.read_msg()? {
        match message {
            AnyMessage::Snapshot(decoder) => {
                let snapshot = SnapshotDomain::try_from_decoder(decoder)?;
                let bids = snapshot
                    .bids
                    .iter()
                    .map(|level| format!("{}:{}", level.price_level, level.quantity))
                    .collect::<Vec<_>>()
                    .join(",");
                let asks = snapshot
                    .asks
                    .iter()
                    .map(|level| format!("{}:{}", level.price_level, level.quantity))
                    .collect::<Vec<_>>()
                    .join(",");
                writeln!(
                    out,
                    "snapshot {}:{}:{}:bids={bids}:asks={asks}",
                    snapshot.symbol,
                    snapshot.received_at.timestamp_micros(),
                    snapshot.exchange_timestamp.timestamp_micros(),
                )?;
            }
            AnyMessage::PriceLevelUpdate(decoder) => {
                let update = PriceLevelUpdateDomain::try_from_decoder(decoder)?;
                writeln!(
                    out,
                    "update {}:{}:{}:side={}:price={}:qty={}",
                    update.symbol,
                    update.backtest_timestamp.timestamp_micros(),
                    update.exchange_timestamp.timestamp_micros(),
                    update.side as u8,
                    update.price_level,
                    update.quantity,
                )?;
            }
            _ => anyhow::bail!("unexpected message"),
        }
    }
    out.flush()?;
    Ok(())
}
