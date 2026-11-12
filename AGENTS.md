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
