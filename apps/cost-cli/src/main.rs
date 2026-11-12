use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(flag), Some(path), None) = (args.next(), args.next(), args.next()) else {
        bail!("usage: cost-cli --schedule path/to/rates.json (JSON lines on stdin)");
    };
    if flag != "--schedule" {
        bail!("usage: cost-cli --schedule path/to/rates.json (JSON lines on stdin)");
    }
    let schedule =
        std::fs::read_to_string(&path).with_context(|| format!("cannot read schedule {path}"))?;
    let book = cost_cli::load_book(&schedule)?;
    let stdout = io::stdout();
    let mut output = io::BufWriter::new(stdout.lock());
    cost_cli::run(&book, io::stdin().lock(), &mut output)?;
    output.flush().context("flush stdout")
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "cost-cli: {error:#}");
            ExitCode::FAILURE
        }
    }
}
