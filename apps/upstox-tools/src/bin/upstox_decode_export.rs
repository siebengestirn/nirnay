//! Decode an immutable Upstox raw-frame JSONL journal into full V3 JSON.
//!
//! This is an offline research preparation tool. It validates each RawFrame,
//! decodes the original protobuf bytes, and preserves capture timestamps,
//! local sequence and source frame SHA-256. It never connects to a broker.
//!
//! The journal is read through the shared bounded `JournalReader` so record
//! limits, torn-tail detection and EOF semantics match the recorder and the
//! rest of the archive. Output is published only after every record succeeds.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::{
    fs::OpenOptions,
    io::{
        BufReader, //
        BufWriter,
        Write,
    },
    path::{
        Path, //
        PathBuf,
    },
};

use anyhow::{
    Context, //
    ensure,
};
use clap::Parser;
use market_archive::JournalReader;
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

fn decode_journal(input: &Path, output: &Path) -> anyhow::Result<u64> {
    ensure!(input.is_file(), "input raw frame journal does not exist");
    ensure!(input != output, "input and output paths must differ");
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent).context("creating output directory")?;
    let output_name = output.file_name().context("output path must name a file")?;
    let temp_path = parent.join(format!(
        ".{}.decode-{}.tmp",
        output_name.to_string_lossy(),
        std::process::id()
    ));

    let decode_result = (|| -> anyhow::Result<u64> {
        let input_file = std::fs::File::open(input).context("opening raw frame journal")?;
        let mut reader = JournalReader::new(BufReader::new(input_file));
        let output_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .context("creating temporary output exclusively")?;
        let mut writer = BufWriter::new(output_file);
        let mut count = 0_u64;

        while let Some(frame) = reader.next_frame().context("reading raw frame journal")? {
            let index = reader.frame_count();
            ensure!(
                frame.source == "upstox",
                "raw frame line {} is not an Upstox frame",
                index
            );
            ensure!(
                frame.received_utc_nanos > 0,
                "raw frame has invalid receive timestamp at line {}",
                index
            );
            let payload = frame
                .payload()
                .with_context(|| format!("verifying source payload SHA-256 at line {}", index))?;
            let decoded = decode_feed_response(&payload)
                .with_context(|| format!("decoding Upstox V3 Protobuf at line {}", index))?;
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
    if let Err(error) = std::fs::hard_link(&temp_path, output) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error).context("publishing decoded output exclusively; refusing to overwrite");
    }
    std::fs::remove_file(&temp_path).context("removing temporary decoded output link")?;
    let directory = std::fs::File::open(parent).context("opening output directory for fsync")?;
    directory
        .sync_all()
        .context("syncing output directory after publication")?;
    Ok(count)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let count = decode_journal(&args.input, &args.output)?;
    eprintln!("decoded_frames={count}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{
        Path, //
        PathBuf,
    };

    use market_archive::RawFrame;

    use super::decode_journal;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("nirnay-decode-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn frame(sequence: u64, payload: &[u8]) -> RawFrame {
        RawFrame::new(
            "upstox",
            "NSE_EQ|TEST",
            sequence as i64,
            sequence,
            None,
            Some(sequence),
            "synthetic-fixture",
            payload,
        )
        .unwrap()
    }

    #[test]
    fn decode_succeeds_for_a_valid_journal() {
        let dir = TestDir::new("valid");
        let input = dir.path().join("raw.jsonl");
        let output = dir.path().join("decoded.jsonl");
        let lines = [
            frame(1, b"").to_json_line().unwrap(),
            frame(2, b"").to_json_line().unwrap(),
        ];
        std::fs::write(&input, format!("{}\n", lines.join("\n"))).unwrap();

        let count = decode_journal(&input, &output).unwrap();

        assert_eq!(count, 2, "both frames decode");
        assert!(output.is_file(), "output is published");
        let content = std::fs::read_to_string(&output).unwrap();
        assert_eq!(content.lines().count(), 2, "one output row per frame");
        assert!(
            content.contains("\"provider\":\"upstox\""),
            "output carries provider metadata"
        );
    }

    #[test]
    fn decode_empty_journal_is_rejected() {
        let dir = TestDir::new("empty");
        let input = dir.path().join("raw.jsonl");
        let output = dir.path().join("decoded.jsonl");
        std::fs::write(&input, b"").unwrap();

        assert!(
            decode_journal(&input, &output).is_err(),
            "empty input is rejected"
        );
        assert!(!output.exists(), "no output is published for empty input");
    }

    #[test]
    fn decode_failure_after_several_records_publishes_nothing() {
        let dir = TestDir::new("late-failure");
        let input = dir.path().join("raw.jsonl");
        let output = dir.path().join("decoded.jsonl");
        let good = frame(1, b"").to_json_line().unwrap();
        let bad = frame(2, b"not protobuf at all").to_json_line().unwrap();
        std::fs::write(&input, format!("{good}\n{bad}\n")).unwrap();

        assert!(
            decode_journal(&input, &output).is_err(),
            "a late decode error aborts the run"
        );
        assert!(!output.exists(), "partial output is not published");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains("decode-"))
            .collect();
        assert!(leftovers.is_empty(), "temporary output is removed");
    }

    #[test]
    fn decode_torn_tail_publishes_nothing() {
        let dir = TestDir::new("torn");
        let input = dir.path().join("raw.jsonl");
        let output = dir.path().join("decoded.jsonl");
        let good = frame(1, b"").to_json_line().unwrap();
        let torn = frame(2, b"").to_json_line().unwrap();
        std::fs::write(&input, format!("{good}\n{torn}")).unwrap();

        assert!(
            decode_journal(&input, &output).is_err(),
            "a torn tail aborts decoding"
        );
        assert!(!output.exists(), "partial output is not published");
    }

    #[test]
    fn decode_rejects_an_existing_output_identity() {
        let dir = TestDir::new("no-overwrite");
        let input = dir.path().join("raw.jsonl");
        let output = dir.path().join("decoded.jsonl");
        let line = frame(1, b"").to_json_line().unwrap();
        std::fs::write(&input, format!("{line}\n")).unwrap();
        std::fs::write(&output, b"existing immutable output").unwrap();

        assert!(
            decode_journal(&input, &output).is_err(),
            "reusing an output identity fails"
        );
        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"existing immutable output",
            "the existing output is not overwritten"
        );
    }
}
