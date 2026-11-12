#![allow(
    clippy::print_stdout,
    reason = "Cargo build scripts emit rerun directives on stdout"
)]

//! Build script: compile the pinned Upstox Market Data Feed V3 schema.
//! No network fetch; the `.proto` is checked in under `schemas/upstox/v3/`.

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // crates/upstox-feed-proto/ -> workspace root is two levels up.
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .ok_or("manifest dir has no workspace parent")?;
    let schema_path = workspace_root.join("schemas/upstox/v3/MarketDataFeedV3.proto");
    let schema_dir = workspace_root.join("schemas/upstox/v3");
    println!("cargo::rerun-if-changed={}", schema_path.display());

    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let include = protoc_bin_vendored::include_path()?;
    let include_str = include.to_str().ok_or("protoc include path is not UTF-8")?;
    let schema_str = schema_path.to_str().ok_or("schema path is not UTF-8")?;
    let schema_dir_str = schema_dir.to_str().ok_or("schema dir is not UTF-8")?;

    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    config.compile_protos(&[schema_str], &[schema_dir_str, include_str])?;
    Ok(())
}
