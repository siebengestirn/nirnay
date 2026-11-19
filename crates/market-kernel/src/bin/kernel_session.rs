#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Offline kernel session projector.
//!
//! Reads a verified kernel session JSON, projects it into canonical kernel
//! events, and publishes the result exclusively with a digest sidecar. No
//! network, broker or order behavior.

use std::{
    fs::File,
    io::{
        BufWriter, //
        Write,
    },
    path::{
        Path, //
        PathBuf,
    },
};

use anyhow::{
    Context, //
    bail,
};
use clap::Parser;
use sha2::{
    Digest, //
    Sha256,
};

use market_kernel::{
    KernelSession, //
    project_session,
};

#[derive(Debug, Parser)]
#[command(
    name = "kernel-session",
    about = "Project a verified source session into canonical kernel events"
)]
struct Args {
    /// Verified kernel session JSON input.
    #[arg(long)]
    input: PathBuf,
    /// New kernel-events JSONL output.
    #[arg(long)]
    output: PathBuf,
}

fn project(input: &Path, output: &Path) -> anyhow::Result<(u64, String)> {
    let bytes = std::fs::read(input).context("reading session input")?;
    let session: KernelSession = serde_json::from_slice(&bytes).context("parsing session input")?;
    let events = project_session(&session).context("projecting session")?;

    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).context("creating output directory")?;
    if output.exists() {
        bail!("output already exists; kernel publication is exclusive");
    }

    let file = File::create_new(output).context("creating kernel event output")?;
    let mut writer = BufWriter::new(file);
    let mut events_written = 0_u64;
    let mut hasher = Sha256::new();
    for event in events {
        let line = serde_json::to_vec(&event).context("serializing kernel event")?;
        hasher.update(&line);
        hasher.update(b"\n");
        writer.write_all(&line).context("writing kernel event")?;
        writer.write_all(b"\n").context("writing newline")?;
        events_written += 1;
    }
    writer.flush().context("flushing output")?;
    writer.get_ref().sync_all().context("syncing output")?;
    let digest = format!("{:x}", hasher.finalize());
    Ok((events_written, digest))
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let (count, digest) = project(&args.input, &args.output)?;
    eprintln!("events={count} sha256={digest}");
    Ok(())
}
