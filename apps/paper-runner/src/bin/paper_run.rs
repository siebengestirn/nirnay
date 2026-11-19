#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Run one bounded paper pass from a JSON input and publish its report.

use std::{
    fs, //
    io::Write,
    path::PathBuf,
};

use paper_runner::PaperInput;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let input = arguments
        .next()
        .ok_or("usage: paper_run <input.json> <output.json> <oms-dir>")?;
    let output = arguments
        .next()
        .ok_or("usage: paper_run <input.json> <output.json> <oms-dir>")?;
    let oms_dir = arguments
        .next()
        .ok_or("usage: paper_run <input.json> <output.json> <oms-dir>")?;

    let bytes = fs::read(&input)?;
    let paper_input: PaperInput = serde_json::from_slice(&bytes)?;
    let oms_path = PathBuf::from(&oms_dir).join("oms.journal");
    if let Some(parent) = oms_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let run =
        paper_runner::run_paper(&paper_input, &oms_path).map_err(|error| error.to_string())?;
    let report = serde_json::to_vec_pretty(&run.report)?;
    let mut file = fs::File::create(&output)?;
    file.write_all(&report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    eprintln!("digest={}", run.report.run_digest);
    Ok(())
}
