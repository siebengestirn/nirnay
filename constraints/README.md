# Resolved Python constraint sets

These files pin the full resolved dependency graph for each independently
installable Python surface, so a resolver change cannot silently alter a release
environment. They are generated with `uv` against the Python 3.12 target that CI
uses, and are checked into version control.

| File | Source manifest | Extras | Used by |
|---|---|---|---|
| `python-research.txt` | `research/python/pyproject.toml` | `dev`, `research`, `security` | `python-research` CI job |
| `python-kite-ops.txt` | `apps/kite-ops/pyproject.toml` | `test` | `python-provider` CI job (kite) |
| `python-fyers-recorder.txt` | `apps/fyers-recorder/pyproject.toml` | `test` | `python-provider` CI job (fyers) |

## Regenerate

```bash
make constraints
```

or run the underlying commands directly:

```bash
uv pip compile research/python/pyproject.toml --extra dev --extra research --extra security \
  --python-version 3.12 --no-emit-package nirnay-quantlab -o constraints/python-research.txt
uv pip compile apps/kite-ops/pyproject.toml --extra test \
  --python-version 3.12 --no-emit-package nirnay-kite-ops -o constraints/python-kite-ops.txt
uv pip compile apps/fyers-recorder/pyproject.toml --extra test \
  --python-version 3.12 --no-emit-package nirnay-fyers-recorder -o constraints/python-fyers-recorder.txt
```

Each file records the exact command in its header. Regeneration is a reviewable
diff: a changed version is a deliberate dependency decision, not an incidental
side effect of installing.

## Verify

```bash
python3 scripts/supply_chain.py constraints
```

The gate fails when a declared dependency is missing from its constraint file, when
a pin violates the declared specifier in `pyproject.toml`, when a line is not an
exact `name==version` pin, or when the generation-command header is absent.

## Scope

The FYERS and Kite constraint sets stay separate on purpose. `fyers-apiv3 3.2.1`
pins `requests==2.31.0`, which conflicts with `nirnay-kite-ops`'s
`requests>=2.32,<3`; see `engineering/provider-environments.md`. The combined
install is tested as an expected failure by the `provider-boundary` CI job.

Constraints pin versions, not wheel hashes. Hashes would add platform coupling
that the current single-platform CI cannot honestly verify; the SBOM plus the
release manifest carry the integrity record instead.

Known advisories inherited from provider SDK pins are waived, with an owner and an
expiry date, in `constraints/pip-audit-allowlist.toml`.
