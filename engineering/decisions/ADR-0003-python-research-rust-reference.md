# ADR-0003: Python for exploration, Rust for authoritative trading rules

**Status:** Accepted.

Python is used for analysis, statistics and experiment orchestration. Rust owns
money, effective-dated fees, risk, order state and the canonical simulation
contracts. Python must call or consume the Rust implementation for financial
accounting rather than reimplementing fees or money arithmetic independently.
