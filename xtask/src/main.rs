#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Repository policy gates, exposed as a discoverable Rust CLI.
//!
//! The task set is a clap subcommand enum: one source of truth for names,
//! help text and delegation. Each gate delegates to `scripts/check_repo.py`;
//! this binary does not duplicate the Makefile task registry or implement
//! policy rules itself. Processes are spawned with explicit argument vectors,
//! never through a shell. The child's exit status is propagated so CI sees
//! the gate's own result.

use std::{
    io::{
        BufWriter, //
        Write,
        stdout,
    },
    path::{
        Path, //
        PathBuf,
    },
    process::{
        Command, //
        ExitCode,
        ExitStatus,
    },
};

use anyhow::{
    Context, //
    Result,
    ensure,
};
use clap::{
    CommandFactory, //
    Parser,
    Subcommand,
};

/// Repository policy gates backed by `scripts/check_repo.py`.
#[derive(Debug, Parser)]
#[command(name = "xtask", version, about, long_about = None)]
struct Cli {
    /// Gate to run.
    #[command(subcommand)]
    task: Task,
}

/// One policy gate or a discovery command.
#[derive(Debug, PartialEq, Eq, Subcommand)]
enum Task {
    /// Run every repository policy gate.
    Check,
    /// Check the architecture dependency graph and manifest hygiene.
    Arch,
    /// Check dependency waivers.
    Waivers,
    /// Check claims and their falsifiers.
    Claims,
    /// List the available tasks.
    List,
}

impl Task {
    /// Argument passed to `check_repo.py`; discovery commands have none.
    fn script_argument(self) -> Option<&'static str> {
        match self {
            Task::Check => Some("check"),
            Task::Arch => Some("arch"),
            Task::Waivers => Some("waivers"),
            Task::Claims => Some("claims"),
            Task::List => None,
        }
    }
}

/// Locate the workspace root from the compiled crate location.
fn workspace_root() -> Result<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .context("xtask must live directly under the workspace root")?
        .to_path_buf();
    ensure!(
        root.join("Cargo.toml").is_file(),
        "workspace manifest not found under {}",
        root.display()
    );
    Ok(root)
}

/// Task names taken from the clap command definition, so they cannot drift.
fn task_names() -> Vec<String> {
    let mut names: Vec<String> = Cli::command()
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_owned())
        .collect();
    names.sort_unstable();
    names
}

/// Map a child status to this process's exit code.
///
/// A successful child stays successful. A failed child keeps its own code when
/// it fits in `u8`; a signal termination or out-of-range code becomes the
/// generic failure code. Keeping this mapping pure makes propagation testable.
fn exit_code_for(status: ExitStatus) -> ExitCode {
    if status.success() {
        return ExitCode::SUCCESS;
    }
    status
        .code()
        .and_then(|value| u8::try_from(value).ok())
        .map_or(ExitCode::FAILURE, ExitCode::from)
}

/// Run one gate and propagate the child's exit code.
fn run_gate(root: &Path, argument: &str) -> Result<ExitCode> {
    let script = root.join("scripts").join("check_repo.py");
    ensure!(
        script.is_file(),
        "repository gate script is missing: {}",
        script.display()
    );
    let status = Command::new("python3")
        .arg(&script)
        .arg(argument)
        .current_dir(root)
        .status()
        .with_context(|| format!("running {}", script.display()))?;
    Ok(exit_code_for(status))
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let root = workspace_root()?;
    match cli.task.script_argument() {
        Some(argument) => run_gate(&root, argument),
        None => {
            let output = stdout();
            let mut writer = BufWriter::new(output.lock());
            for name in task_names() {
                writeln!(writer, "{name}").context("writing the task list")?;
            }
            writer.flush().context("flushing the task list")?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("xtask: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_policy_gate_maps_to_a_script_argument() {
        assert_eq!(
            Task::Check.script_argument(),
            Some("check"),
            "check delegates to the full gate"
        );
        assert_eq!(
            Task::Arch.script_argument(),
            Some("arch"),
            "arch delegates to the architecture gate"
        );
        assert_eq!(
            Task::Waivers.script_argument(),
            Some("waivers"),
            "waivers delegates to the waiver gate"
        );
        assert_eq!(
            Task::Claims.script_argument(),
            Some("claims"),
            "claims delegates to the claims gate"
        );
    }

    #[test]
    fn discovery_task_has_no_script_argument() {
        assert_eq!(
            Task::List.script_argument(),
            None,
            "list does not invoke a policy script"
        );
    }

    #[test]
    fn listed_tasks_come_from_the_cli_definition() {
        assert_eq!(
            task_names(),
            vec!["arch", "check", "claims", "list", "waivers"],
            "the task list reflects the subcommand enum"
        );
    }

    #[test]
    fn each_subcommand_parses_to_its_task() {
        for (input, expected) in [
            ("check", Task::Check),
            ("arch", Task::Arch),
            ("waivers", Task::Waivers),
            ("claims", Task::Claims),
            ("list", Task::List),
        ] {
            let cli = Cli::try_parse_from(["xtask", input])
                .unwrap_or_else(|error| panic!("parsing {input} failed: {error}"));
            assert_eq!(cli.task, expected, "subcommand {input} parses");
        }
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(
            Cli::try_parse_from(["xtask", "not-a-task"]).is_err(),
            "an unknown task is refused rather than forwarded to a script"
        );
    }

    fn child_status(code: i32) -> ExitStatus {
        Command::new("python3")
            .args(["-c", &format!("raise SystemExit({code})")])
            .status()
            .expect("spawn status probe")
    }

    #[test]
    fn successful_child_stays_successful() {
        assert_eq!(
            format!("{:?}", exit_code_for(child_status(0))),
            format!("{:?}", ExitCode::SUCCESS),
            "a passing gate exits successfully"
        );
    }

    #[test]
    fn failing_child_exit_code_is_propagated() {
        assert_eq!(
            format!("{:?}", exit_code_for(child_status(7))),
            format!("{:?}", ExitCode::from(7u8)),
            "the gate's own exit code reaches CI"
        );
    }

    #[test]
    fn delegated_arguments_are_accepted_by_the_gate_script() {
        let root = workspace_root().expect("workspace root");
        let script = root.join("scripts").join("check_repo.py");
        for (task, argument) in [
            (Task::Check, "check"),
            (Task::Arch, "arch"),
            (Task::Waivers, "waivers"),
            (Task::Claims, "claims"),
        ] {
            assert_eq!(
                task.script_argument(),
                Some(argument),
                "the CLI mapping matches the gate name"
            );
            let status = Command::new("python3")
                .arg(&script)
                .arg(argument)
                .current_dir(&root)
                .status()
                .expect("run gate script");
            assert!(
                status.success(),
                "gate `{argument}` is accepted and passes in the current tree"
            );
        }
    }
}
