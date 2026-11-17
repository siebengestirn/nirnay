# Source style policy

This repository applies one binding rule to every language: **formatters and
linters run on all files; style guidance is enforced on lines added relative to
`main`**. That keeps existing code from being churned while making every new
line comply. The rule is implemented by `scripts/check_rust_style.py` for Rust
and `scripts/check_source_style.py` for every other language, and is wired into
`make check` and CI.

## Sources and conflict resolution

- **Rust**: Linux kernel Rust coding guidelines and Google Rust style guide,
  with the repository's resolution (one `use` per module, kernel vertical
  import layout with a trailing empty comment). No exceptions.
- **Python**: Google Python Style Guide. We keep its import, exception,
  semicolon, backslash-continuation and mutable-default rules. The repository
  uses a 100-column limit (`.editorconfig`, `.ruff.toml`) rather than Google's
  80; the 100-column convention is already established here.
- **C/C++**: Google C++ Style Guide, expressed by the pinned `.clang-format`
  (LLVM base, two-space indent, 100 columns, `nullptr`, regioned includes).
  The Linux kernel C style (eight-space tabs) is deliberately **not** used for
  application C++; it is a kernel constraint, not a repository convention.
- **Shell**: Google Shell Style Guide. Executable scripts start with
  `#!/bin/bash`, use two-space indentation, stay within 80 columns, prefer
  `[[ ... ]]`, `$(...)`, `((...))` and `"$@"`, and avoid `eval`, backticks,
  `let`, `expr`, `test`, `/usr/bin/[`, aliases and unquoted `$*`. Shell keeps
  Google's 80-column limit while Python, C/C++ and Markdown retain the
  repository's 100-column convention. Extensionless scripts with a shell
  shebang are detected; a file with no shebang and no recognised extension is
  not classified as shell.
- **Makefile**: GNU Make semantics plus the repository's tab-indented recipe
  rule in `.editorconfig`.
- **JSON/JSONL**: Google JSON Style Guide constraints that are mechanical:
  valid JSON only (no comments), double-quoted keys and strings, two-space
  indentation, a final newline. Semantic naming rules are not enforced because
  existing schema property names are part of published contracts.
- **Markdown**: Google Markdown style guide — ATX headings with a space after
  `#`, a single H1, fenced blocks with a declared language, no trailing
  whitespace, and a 100-column prose limit with exceptions for headings,
  tables, links and code blocks.

A web search for published style guides from leading HFT/quantitative trading
firms (Jane Street, Citadel, Two Sigma, HRT, Optiver) found no authoritative
public coding-style documents; results were interview-preparation material.
We therefore do not attribute any rule to those firms. The low-latency
discipline in our own ADRs (allocation control, no filesystem durability on a
hot path, exact financial types) remains the binding domain constraint and is
enforced by code and tests, not by a formatter.

## Binding tools

| Language | Formatter/linter | Added-lines gate | Where it runs |
|---|---|---|---|
| Rust | `cargo fmt`, Clippy `-D warnings` | `check_rust_style.py` | `make check`, CI |
| Python | `ruff` (advisory, see below) | `check_source_style.py` | `make check`, CI |
| C/C++ | pinned `clang-format==21.1.8` | `check_source_style.py` | `make check`, CI |
| Shell | none available | `check_source_style.py` | `make check`, CI |
| Makefile | none available | `check_source_style.py` | `make check`, CI |
| JSON/JSONL | none needed | `check_source_style.py` | `make check`, CI |
| Markdown | none available | `check_source_style.py` | `make check`, CI |

Run the gates directly:

```sh
make style
python3 scripts/check_rust_style.py --self-test
python3 scripts/check_source_style.py --self-test
python3 scripts/check_style_gate.py --self-test
```

`ruff` is configured in `.ruff.toml` and available via `make python-format-check`,
but is **not binding** yet: roughly 59 existing files predate it and would be
reformatted (measured with ruff 0.16.5; the exact count is version-dependent).
A dedicated PR must reformat the corpus before whole-tree ruff checks can join
`make check`. Until then the dependency-free added-lines gate is the
authoritative Python style check. The new checkers themselves are ruff-formatted.

## Hardening the gate itself

`scripts/check_style_gate.py` is a binding integrity check that verifies:

- `make check` still invokes `style`, and `style` still invokes both style
  checkers and the integrity checker;
- the CI `source-style` job still runs both checkers with full history;
- all six language check functions still exist in `check_source_style.py`
  (Rust is checked by `check_rust_style.py`);
- the adversarial self-test inventory has not shrunk below its declared
  minimum (13 Rust, 26 source-style, 7 integrity tests);
- the integrity checker is invoked twice in CI and once in the Makefile;
- canonical bad lines still produce violations, so a rule whose body is
  replaced with `return []` fails the gate even if its name and tests remain.

The behavioural self-tests also run explicitly in the CI `source-style` job.
The integrity checker runs from `make style`, from that CI job and
independently from the CI `python-research` job. Removing or weakening a rule,
a checker, a self-test or the wiring therefore fails the build. Adding a
language or rule requires updating its expectations in the same PR.

## Adding a rule

Add the rule and an adversarial self-test to the relevant checker, document the
policy source here, raise the minimum in `check_style_gate.py` when applicable,
then run `make check`. A rule that cannot be expressed on added lines without
churning existing code does not belong in the binding gate.
