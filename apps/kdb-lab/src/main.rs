#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
use std::io::{
    self, //
    Write,
};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{
    Context, //
    Result,
};
use clap::{
    Args, //
    Parser,
    Subcommand,
};

use kdb_lab::{
    MbpMetrics, analyze, build_report, find_q, golden_dir, parse_metrics_csv,
    parse_observations_csv, reference_metrics,
};

#[derive(Debug, Args)]
struct AnalyzeArgs {
    /// Header-prefixed MBP observation CSV.
    #[arg(long)]
    input: PathBuf,
    /// Source identifier, echoed by the q contract.
    #[arg(long)]
    source: String,
    /// Output directory for metrics.csv and report.json.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the checked-in KDB-X analytics contract on one source.
    Analyze(AnalyzeArgs),
    /// Validate the golden contract without requiring a q installation.
    CheckGolden,
}

#[derive(Debug, Parser)]
#[command(name = "nirnay-kdb", about = "Nirnay KDB-X analytics contract harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

fn analyze_command(args: AnalyzeArgs) -> Result<()> {
    let q_bin = find_q()?;
    let result = analyze(&args.input, &args.source, &q_bin)?;
    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("cannot create {}", args.out.display()))?;
    let metrics_path = args.out.join("metrics.csv");
    let report_path = args.out.join("report.json");
    std::fs::write(&metrics_path, result.stdout.as_bytes())
        .with_context(|| format!("cannot write {}", metrics_path.display()))?;
    let report = build_report(&result, &args.input.to_string_lossy());
    let report_text = serde_json::to_string_pretty(&report).context("cannot encode report")?;
    std::fs::write(&report_path, format!("{report_text}\n"))
        .with_context(|| format!("cannot write {}", report_path.display()))?;
    eprintln!(
        "kdb-lab: source={} rows={} input_sha256={} output_sha256={}",
        result.metrics.source_id, result.input_rows, result.input_sha256, result.output_sha256
    );
    eprintln!(
        "kdb-lab: wrote {} and {}",
        metrics_path.display(),
        report_path.display()
    );
    Ok(())
}

fn metrics_close(actual: &MbpMetrics, expected: &MbpMetrics) -> bool {
    let integers_match = actual.schema_version == expected.schema_version
        && actual.source_id == expected.source_id
        && actual.rows == expected.rows
        && actual.gap_count == expected.gap_count
        && actual.levels_min == expected.levels_min
        && actual.bid1_max_qty == expected.bid1_max_qty
        && actual.ask1_max_qty == expected.ask1_max_qty;
    let floats = [
        (actual.duration_s, expected.duration_s),
        (actual.spread_mean, expected.spread_mean),
        (actual.spread_median, expected.spread_median),
        (actual.spread_max, expected.spread_max),
        (actual.imbalance_mean, expected.imbalance_mean),
        (actual.imbalance_min, expected.imbalance_min),
        (actual.imbalance_max, expected.imbalance_max),
        (actual.bid1_mean, expected.bid1_mean),
        (actual.ask1_mean, expected.ask1_mean),
    ];
    integers_match
        && floats
            .into_iter()
            .all(|(left, right)| (left - right).abs() <= 1e-6)
}

fn check_golden() -> Result<()> {
    let dir = golden_dir();
    let input_path = dir.join("mbp-observation-v1.csv");
    let metrics_path = dir.join("mbp-metrics-v1.csv");
    let input_text = std::fs::read_to_string(&input_path)
        .with_context(|| format!("cannot read {}", input_path.display()))?;
    let golden_text = std::fs::read_to_string(&metrics_path)
        .with_context(|| format!("cannot read {}", metrics_path.display()))?;
    let rows = parse_observations_csv(&input_text).context("golden input is invalid")?;
    let expected = parse_metrics_csv(&golden_text).context("golden metrics are invalid")?;
    let reference = reference_metrics(&rows, &expected.source_id)
        .context("reference analytics failed on golden input")?;
    if !metrics_close(&reference, &expected) {
        anyhow::bail!("reference metrics differ from the recorded q golden");
    }
    eprintln!(
        "kdb-lab: golden contract valid (rows={}, source={})",
        expected.rows, expected.source_id
    );
    match find_q() {
        Ok(q_bin) => {
            let result = analyze(&input_path, &expected.source_id, &q_bin)
                .context("fresh q replay of the golden contract failed")?;
            if result.stdout != golden_text {
                anyhow::bail!("fresh q output is not byte-identical to the golden file");
            }
            eprintln!("kdb-lab: fresh q replay is byte-identical");
        }
        Err(error) => {
            eprintln!("kdb-lab: q replay skipped ({error})");
        }
    }
    Ok(())
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Analyze(args) => analyze_command(args),
        Command::CheckGolden => check_golden(),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "nirnay-kdb: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn analyze_subcommand_parses() {
        let cli = Cli::try_parse_from([
            "nirnay-kdb",
            "analyze",
            "--input",
            "book.csv",
            "--source",
            "upstox-live",
            "--out",
            "var/kdb",
        ])
        .expect("analyze arguments must parse");
        assert!(
            matches!(cli.command, super::Command::Analyze(_)),
            "analyze variant"
        );
    }

    #[test]
    fn missing_required_argument_fails() {
        assert!(
            Cli::try_parse_from(["nirnay-kdb", "analyze"]).is_err(),
            "analyze requires --input, --source and --out"
        );
    }
}
