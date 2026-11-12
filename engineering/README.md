# Engineering control plane

Machine-readable source files live here. The README is an orientation document,
not a second status database. Change `capabilities.json`, the relevant ADR and
`claims/ledger.json` only when the underlying ownership, scope or evidence changes.

- `architecture/layers.toml`: dependency rules checked against Cargo metadata.
- `capabilities.json`: current implementation state; `implemented` is not `verified`.
- `decisions/`: accepted architecture decisions and their consequences.
- `verification-policy.md`: what constitutes evidence.
- `strategy-validation-policy.md`: pre-registration and research gates.
