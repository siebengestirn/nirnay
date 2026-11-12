# Contributing

1. Make one coherent change per pull request.
2. Add tests with the implementation; include negative and failure-path tests.
3. Run formatting, `cargo xtask check`, Rust tests/clippy and Python tests.
4. For data changes, attach provenance, entitlement and quality evidence.
5. For research changes, preregister the experiment and preserve all trials.
6. For any execution change, prove risk cannot be bypassed and broker state can be reconciled.
7. Update architecture decisions only when a decision changes; never use status prose as a substitute for tests.

No live secrets, personal account identifiers, licensed market data or generated
local datasets belong in pull requests.
