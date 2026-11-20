#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Replay a verified kernel session into per-source books and publish a
//! deterministic reconciliation report. No provider connection, no live path.

use std::{
    collections::{
        BTreeMap, //
        BTreeSet,
    },
    io::Write,
    path::PathBuf,
};

use anyhow::{
    Context, //
    bail,
};
use clap::Parser;
use market_book::{
    MbpBook, //
    SnapshotEvidence,
    compare_books,
};
use market_event::MarketDataEvent;
use market_kernel::{
    KernelSession, //
    project_session,
};
use serde::Serialize;
use sha2::{
    Digest, //
    Sha256,
};

#[derive(Debug, Parser)]
#[command(
    name = "session-replay",
    about = "Replay a verified kernel session into per-source books"
)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
struct ReplayReport {
    schema_version: u32,
    events: usize,
    sources: Vec<String>,
    books: Vec<BookLine>,
    comparison: Option<ComparisonLine>,
    #[serde(skip)]
    digest: String,
}

#[derive(Clone, Debug, Serialize)]
struct BookLine {
    source_id: String,
    phase: String,
    best_bid_paise: Option<i64>,
    best_ask_paise: Option<i64>,
    bid_levels: usize,
    ask_levels: usize,
}

#[derive(Clone, Debug, Serialize)]
struct ComparisonLine {
    left: String,
    right: String,
    matched: bool,
    differences: Vec<String>,
}

fn replay(input: &PathBuf) -> anyhow::Result<ReplayReport> {
    let bytes = std::fs::read(input).context("reading kernel session")?;
    let session: KernelSession =
        serde_json::from_slice(&bytes).context("parsing kernel session")?;
    let events = project_session(&session).context("projecting session")?;

    let mut books: BTreeMap<String, MbpBook> = BTreeMap::new();
    let mut seen_sources = BTreeSet::new();

    for event in &events {
        seen_sources.insert(event.source_id.clone());
        let Some(MarketDataEvent::MarketByPriceSnapshot(snapshot)) = &event.payload else {
            continue;
        };
        if event.quality != market_event::StreamState::Clean {
            continue;
        }
        let book = match books.entry(event.source_id.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let created = MbpBook::new(&event.source_id, snapshot.depth_levels)
                    .context("creating per-source book")?;
                entry.insert(created)
            }
        };
        let evidence = SnapshotEvidence {
            source_id: event.source_id.clone(),
            snapshot_id: event.observation_id.to_string(),
            source_digest: event.raw_frame_hash.to_string(),
            received_utc_ns: event.receive_utc_ns,
        };
        if book.phase() == market_book::BookPhase::Uninitialized {
            book.begin_snapshot(&evidence)
                .context("beginning snapshot")?;
        } else if book.phase() == market_book::BookPhase::Gap {
            book.begin_recovery().context("beginning recovery")?;
            book.begin_snapshot(&evidence)
                .context("beginning recovery snapshot")?;
        } else {
            continue;
        }
        let book_snapshot = snapshot_to_book_snapshot(snapshot)?;
        book.apply_snapshot(&book_snapshot, &evidence)
            .context("applying snapshot")?;
        book.mark_live().context("marking book live")?;
    }

    let lines: Vec<BookLine> = books
        .values()
        .map(|book| {
            let snapshot = book.to_snapshot();
            BookLine {
                source_id: book.source_id().to_owned(),
                phase: format!("{:?}", book.phase()),
                best_bid_paise: book.best_bid().map(|level| level.price.raw()),
                best_ask_paise: book.best_ask().map(|level| level.price.raw()),
                bid_levels: snapshot.bids.len(),
                ask_levels: snapshot.asks.len(),
            }
        })
        .collect();

    let mut values = books.values();
    let pair = values.next().zip(values.next());
    let comparison = if let Some((left, right)) = pair {
        let result = compare_books(left, right);
        Some(ComparisonLine {
            left: result.left.source_id,
            right: result.right.source_id,
            matched: result.matched,
            differences: result.differences,
        })
    } else {
        None
    };

    Ok(ReplayReport {
        schema_version: 1,
        events: events.len(),
        sources: seen_sources.into_iter().collect(),
        books: lines,
        comparison,
        digest: String::new(),
    })
}

fn snapshot_to_book_snapshot(
    snapshot: &market_event::MarketByPriceSnapshot,
) -> anyhow::Result<market_types::model::BookSnapshot> {
    let convert = |level: &market_event::MbpLevel| -> anyhow::Result<market_types::model::Level> {
        market_types::model::Level::new(level.price_paise, level.quantity)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    };
    Ok(market_types::model::BookSnapshot {
        bids: snapshot
            .bids
            .iter()
            .map(convert)
            .collect::<anyhow::Result<Vec<_>>>()?,
        asks: snapshot
            .asks
            .iter()
            .map(convert)
            .collect::<anyhow::Result<Vec<_>>>()?,
    })
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.input == args.output {
        bail!("input and output paths must differ");
    }
    let mut report = replay(&args.input)?;
    let preimage = serde_json::to_vec(&report).context("serializing report")?;
    report.digest = format!("{:x}", Sha256::digest(&preimage));
    let mut file = std::fs::File::create(&args.output).context("creating report")?;
    file.write_all(serde_json::to_string_pretty(&report)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    eprintln!(
        "events={} sources={} digest={}",
        report.events,
        report.sources.len(),
        report.digest
    );
    Ok(())
}
