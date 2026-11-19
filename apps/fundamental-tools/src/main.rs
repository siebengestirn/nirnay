use std::fs::File;
use std::io::BufReader;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{
    Parser, //
    Subcommand,
}; //
use fundamental_tools::{
    ValidationLimits, //
    validate_company_jsonl,
    validate_jsonl,
}; //

#[derive(Debug, Parser)]
#[command(
    name = "nirnay-fundamental",
    version,
    about = "Validate Nirnay financial fact datasets"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a complete, LF-terminated JSONL financial-fact dataset.
    ValidateFacts {
        /// Input JSONL file. The file is read but never modified.
        #[arg(long)]
        input: PathBuf,
        /// Maximum bytes per JSON record, including the LF terminator.
        #[arg(long, default_value_t = fundamental_tools::DEFAULT_MAX_RECORD_BYTES)]
        max_record_bytes: usize,
        /// Maximum number of facts in the dataset.
        #[arg(long, default_value_t = fundamental_tools::DEFAULT_MAX_RECORDS)]
        max_records: u64,
        /// Maximum total input bytes. The validator retains typed records in memory.
        #[arg(long, default_value_t = fundamental_tools::DEFAULT_MAX_TOTAL_BYTES)]
        max_total_bytes: u64,
    },
    /// Validate effective-dated, source-backed company identity JSONL.
    ValidateCompanies {
        /// Input JSONL file. The file is read but never modified.
        #[arg(long)]
        input: PathBuf,
        /// Maximum bytes per JSON record, including the LF terminator.
        #[arg(long, default_value_t = fundamental_tools::DEFAULT_MAX_RECORD_BYTES)]
        max_record_bytes: usize,
        /// Maximum number of records in the dataset.
        #[arg(long, default_value_t = fundamental_tools::DEFAULT_MAX_RECORDS)]
        max_records: u64,
        /// Maximum total input bytes. The validator retains typed records in memory.
        #[arg(long, default_value_t = fundamental_tools::DEFAULT_MAX_TOTAL_BYTES)]
        max_total_bytes: u64,
    },
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::ValidateFacts {
            input,
            max_record_bytes,
            max_records,
            max_total_bytes,
        } => {
            let file = File::open(&input)?;
            let report = validate_jsonl(
                BufReader::new(file),
                ValidationLimits {
                    max_record_bytes,
                    max_records,
                    max_total_bytes,
                },
            )?;
            write_report(&report)?;
        }
        Command::ValidateCompanies {
            input,
            max_record_bytes,
            max_records,
            max_total_bytes,
        } => {
            let file = File::open(&input)?;
            let report = validate_company_jsonl(
                BufReader::new(file),
                ValidationLimits {
                    max_record_bytes,
                    max_records,
                    max_total_bytes,
                },
            )?;
            write_report(&report)?;
        }
    }
    Ok(())
}

fn write_report(
    report: &fundamental_tools::ValidationReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let encoded = serde_json::to_vec(report)?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&encoded)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr().lock(), "nirnay-fundamental: {error}");
            ExitCode::FAILURE
        }
    }
}
