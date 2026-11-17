//! Verify a completed Upstox capture session before any research use.
//!
//! This command checks the session manifest, the referenced journal's
//! whole-file hash and frame count, and every journal record through the
//! shared bounded reader. It never connects to a broker and never modifies or
//! repairs the session files.

use std::path::{
    PathBuf, //
};

use anyhow::{
    Context, //
};
use clap::Parser;
use market_archive::verify_capture_session;

#[derive(Debug, Parser)]
#[command(
    name = "upstox-verify-session",
    about = "Verify a completed capture session against its raw journal"
)]
struct Args {
    /// Path to the session manifest.json.
    #[arg(long)]
    manifest: PathBuf,
    /// Provider the manifest must declare.
    #[arg(long, default_value = "Upstox")]
    provider: String,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let result = verify_capture_session(&args.manifest, &args.provider)
        .context("verifying capture session")?;
    eprintln!(
        "session_verified=true provider={} frames={} bytes={} sha256={}",
        args.provider, result.frame_count, result.bytes_scanned, result.sha256_hex
    );
    Ok(())
}
