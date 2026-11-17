# Agent and contributor rules

- Read `README.md`, `engineering/verification-policy.md` and the relevant ADR first.
- Do not claim profitability, live readiness, broker support or data rights without evidence.
- Do not add provider/exchange observations, credentials or account identifiers to Git.
- Financial money, prices, charges and P&L stay in exact Rust domain types; Python is for research statistics.
- A strategy emits intents only. It never accesses broker SDKs, OMS internals, risk permits, cash or positions.
- Do not add a new crate, language, database or service without a contract, test and measured reason.
- Write the hypothesis and falsifier before parameter search; preserve failed experiments.
- Keep live trading disabled. A passing CI run cannot authorize live routing.
- Update `engineering/capabilities.json` and `claims/ledger.json` truthfully; implemented is not verified.
- All Rust changes must follow the Linux kernel Rust coding guidelines (`https://docs.kernel.org/rust/coding-guidelines.html`) and the Google Rust style guide (`https://google.github.io/styleguide/rustguide.html`) without exception. Where the two conflict, satisfy both: one module per `use` statement plus kernel vertical import layout with the trailing empty comment. `cargo fmt`, Clippy `-D warnings` and `python3 scripts/check_rust_style.py` (part of `make check`) are binding for new Rust lines.
