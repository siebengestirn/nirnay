# xtask CLI upgrade — 2026-11-19

**Scope:** `xtask/**` and a contributor note. No new crate, language, service or
dependency source; `clap` was already a workspace dependency. The existing
`cargo xtask check` invocation used by the Makefile and CI is preserved.

## Baseline

`xtask/src/main.rs` was a 22-line wrapper that took an unvalidated positional
string, forwarded it to `python3 scripts/check_repo.py`, and collapsed any child
failure into a generic error. It had no help, no version, no task discovery, no
validation of task names, and no way to observe the gate's own exit code.

## Research basis

- matklad, *cargo xtask* specification
  (<https://github.com/matklad/cargo-xtask>): a single Rust binary with tasks as
  subcommands, no shell, minimal compile time, committed as a workspace member
  with a cargo alias. The repository already follows the workspace and alias
  parts; this change completes the subcommand part.
- clap 4.6 derive reference (<https://docs.rs/clap/latest/clap/_derive/>):
  `Parser` plus `Subcommand`, `#[command(name, version, about)]`, doc-comment
  help, and `CommandFactory::command` with `debug_assert` for validating the
  command definition in a test.

## What changed

- `xtask/src/main.rs`: clap-derived `Cli` with `check`, `arch`, `waivers`,
  `claims` and `list` subcommands. The subcommand enum is the single source of
  truth for names, help text and gate delegation; `list` derives its output
  from the clap command definition, so it cannot drift from the enum.
- Child processes use explicit argument vectors through `Command`, never a
  shell. `workspace_root()` is validated against the workspace manifest.
- `exit_code_for` is a pure mapping from `ExitStatus` to `ExitCode`. A passing
  gate stays passing; a failing gate keeps its own code when it fits in `u8`;
  signal termination or an out-of-range code becomes the generic failure code.
- No production lint suppression was added. The workspace denies
  `print_stdout`; the discovery command writes through a locked `BufWriter`, so
  that invariant is preserved rather than waived. The file keeps only the
  repository's standard test-only `cfg_attr` allow for `unwrap`, `expect` and
  `panic` in unit tests; that attribute is inactive in production and weakens
  no gate.
- `run() -> Result<ExitCode>` holds the fallible work; `main` only converts an
  error into a failure exit code, so `?` is used without suppressing errors.
- `xtask/Cargo.toml` gains the workspace `clap` dependency.
- `CONTRIBUTING.md` documents discovery via `cargo xtask --help`.

## Boundaries

This is a tooling upgrade, not a new task registry. The Makefile remains the
canonical full build surface; xtask continues to own only the repository policy
gates it already wrapped. No policy rule was moved, weakened or duplicated. No
capability or claim state changes. No owner-owned file was edited.

## Tests

Exit-code mapping: success and code 7 are tested with real child processes.
Signal termination is not directly tested because stable Rust cannot construct
an `ExitStatus` for a signal portably; that branch is reviewed rather than
claimed as tested.

Nine tests: command-definition validity (`debug_assert`), each gate's
script-argument mapping, discovery having no script argument, the derived task
list, parsing every subcommand, rejection of an unknown subcommand, real-process
tests that a child's success and a child's exit code 7 propagate correctly, and
an integration test that runs all four delegated arguments against the real
`check_repo.py`. The last test is the drift guard: rename or remove a gate in
the Python script and the Rust CLI test fails instead of silently desynchronising.

## Verification

`cargo test -p xtask` — 9 passed in 0.22 s. Manual checks: `cargo xtask --help`
lists all tasks, `cargo xtask list` prints the five names, `cargo xtask check`
and `cargo xtask arch` PASS, and an unknown task exits 2. `cargo fmt`, Clippy
`-D warnings`, Rust style, source style, prose style and `make check` PASS.
Hosted exact-head CI is recorded by the PR.
