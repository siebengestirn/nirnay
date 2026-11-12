//! Decode an immutable Upstox raw-frame JSONL journal into full V3 JSON.
//!
//! This is an offline research preparation tool. It validates each RawFrame,
//! decodes the original protobuf bytes, and preserves capture timestamps,
//! local sequence and source frame SHA-256. It never connects to a broker.

use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, BufWriter, Write},
    path::PathBuf,
};

use anyhow::{Context, bail, ensure};
use clap::Parser;
use market_archive::RawFrame;
use serde_json::{Value, json};
use upstox_feed_proto::{
    decode::decode_feed_response,
    proto::{
        Feed, FeedResponse, FirstLevelWithGreeks, IndexFullFeed, Ltpc, MarketFullFeed, Quote,
        feed::FeedUnion, full_feed::FullFeedUnion,
    },
};

#[derive(Debug, Parser)]
#[command(
    name = "upstox-decode-export",
    about = "Offline decode of Upstox raw-frame JSONL journal"
)]
struct Args {
    /// Input raw-frames.jsonl produced by upstox-recorder.
    #[arg(long)]
    input: PathBuf,
    /// New output file containing full decoded message envelopes.
    #[arg(long)]
    output: PathBuf,
}

fn ltpc_value(value: &Ltpc) -> Value {
    json!({
        "ltp": value.ltp,
        "ltt": value.ltt.to_string(),
        "ltq": value.ltq.to_string(),
        "cp": value.cp,
        "iep": value.iep,
    })
}

fn quote_value(value: &Quote) -> Value {
    json!({
        "bidP": value.bid_p,
        "bidQ": value.bid_q.to_string(),
        "askP": value.ask_p,
        "askQ": value.ask_q.to_string(),
    })
}

fn market_full_feed_value(value: &MarketFullFeed) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(ltpc) = value.ltpc.as_ref() {
        out.insert("ltpc".to_owned(), ltpc_value(ltpc));
    }
    if let Some(level) = value.market_level.as_ref() {
        out.insert(
            "marketLevel".to_owned(),
            json!({
                "bidAskQuote": level.bid_ask_quote.iter().map(quote_value).collect::<Vec<_>>(),
            }),
        );
    }
    out.insert("vtt".to_owned(), json!(value.vtt.to_string()));
    Value::Object(out)
}

fn index_full_feed_value(value: &IndexFullFeed) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(ltpc) = value.ltpc.as_ref() {
        out.insert("ltpc".to_owned(), ltpc_value(ltpc));
    }
    Value::Object(out)
}

fn first_level_value(value: &FirstLevelWithGreeks) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(ltpc) = value.ltpc.as_ref() {
        out.insert("ltpc".to_owned(), ltpc_value(ltpc));
    }
    if let Some(depth) = value.first_depth.as_ref() {
        out.insert("firstDepth".to_owned(), quote_value(depth));
    }
    out.insert("vtt".to_owned(), json!(value.vtt.to_string()));
    Value::Object(out)
}

fn feed_value(feed: &Feed) -> Value {
    match feed.feed_union.as_ref() {
        Some(FeedUnion::Ltpc(value)) => json!({ "ltpc": ltpc_value(value) }),
        Some(FeedUnion::FirstLevelWithGreeks(value)) => {
            json!({ "firstLevelWithGreeks": first_level_value(value) })
        }
        Some(FeedUnion::FullFeed(value)) => match value.full_feed_union.as_ref() {
            Some(FullFeedUnion::MarketFf(market)) => {
                json!({ "fullFeed": { "marketFF": market_full_feed_value(market) } })
            }
            Some(FullFeedUnion::IndexFf(index)) => {
                json!({ "fullFeed": { "indexFF": index_full_feed_value(index) } })
            }
            None => json!({ "fullFeed": {} }),
        },
        None => json!({}),
    }
}

fn response_value(response: &FeedResponse) -> Value {
    let mut feeds = serde_json::Map::new();
    let mut keys: Vec<_> = response.feeds.keys().collect();
    keys.sort();
    for key in keys {
        if let Some(feed) = response.feeds.get(key) {
            feeds.insert(key.clone(), feed_value(feed));
        }
    }
    let response_type = match response.r#type {
        0 => "initial_feed",
        1 => "live_feed",
        2 => "market_info",
        _ => "unknown",
    };
    json!({
        "type": response_type,
        "currentTs": response.current_ts.to_string(),
        "feeds": Value::Object(feeds),
    })
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    ensure!(
        args.input.is_file(),
        "input raw frame journal does not exist"
    );
    ensure!(
        args.input != args.output,
        "input and output paths must differ"
    );
    let parent = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent).context("creating output directory")?;
    let output_name = args
        .output
        .file_name()
        .context("output path must name a file")?;
    let temp_path = parent.join(format!(
        ".{}.decode-{}.tmp",
        output_name.to_string_lossy(),
        std::process::id()
    ));

    let decode_result = (|| -> anyhow::Result<u64> {
        let input = std::fs::File::open(&args.input).context("opening raw frame journal")?;
        let reader = BufReader::new(input);
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .context("creating temporary output exclusively")?;
        let mut writer = BufWriter::new(output);
        let mut count = 0_u64;

        for (index, line) in reader.lines().enumerate() {
            let line = line.with_context(|| format!("reading input line {}", index + 1))?;
            if line.trim().is_empty() {
                bail!("raw frame journal contains an empty line at {}", index + 1);
            }
            let frame = RawFrame::from_json_line(&line)
                .with_context(|| format!("validating raw frame at line {}", index + 1))?;
            ensure!(
                frame.source == "upstox",
                "raw frame line {} is not an Upstox frame",
                index + 1
            );
            ensure!(
                frame.received_utc_nanos > 0,
                "raw frame has invalid receive timestamp at line {}",
                index + 1
            );
            let payload = frame
                .payload()
                .context("verifying source payload SHA-256")?;
            let decoded = decode_feed_response(&payload)
                .with_context(|| format!("decoding Upstox V3 Protobuf at line {}", index + 1))?;
            let row = json!({
                "schema_version": 1,
                "provider": "upstox",
                "received_at_utc_ns": frame.received_utc_nanos,
                "received_monotonic_ns": frame.received_monotonic_nanos,
                "capture_sequence": frame.capture_sequence,
                "payload_sha256": frame.payload_sha256,
                "message": response_value(&decoded),
            });
            serde_json::to_writer(&mut writer, &row)?;
            writer.write_all(b"\n")?;
            count = count
                .checked_add(1)
                .context("decoded frame count overflow")?;
        }

        writer.flush()?;
        writer.get_ref().sync_all()?;
        ensure!(count > 0, "input raw frame journal contained no records");
        Ok(count)
    })();

    let count = match decode_result {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(error);
        }
    };
    // Hard-link publication is atomic and fails rather than overwriting an
    // existing destination. Temp and destination are siblings on one filesystem.
    if let Err(error) = std::fs::hard_link(&temp_path, &args.output) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error).context("publishing decoded output exclusively; refusing to overwrite");
    }
    std::fs::remove_file(&temp_path).context("removing temporary decoded output link")?;
    let directory = std::fs::File::open(parent).context("opening output directory for fsync")?;
    directory
        .sync_all()
        .context("syncing output directory after publication")?;
    eprintln!("decoded_frames={count}");
    Ok(())
}
