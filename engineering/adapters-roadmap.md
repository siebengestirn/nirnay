# Provider adapter status — current research/paper candidate

The active workspace provides two market-data capture paths and a constrained Kite operator surface. These are not a consolidated exchange feed and do not make the OMS production-certified.

| Provider | Active path | What it does | Current limits / next work |
|---|---|---|---|
| Upstox | `apps/upstox-tools` + `crates/upstox-feed-proto` | V3 authorisation, binary Protobuf capture, diagnostic projections and `upstox-decode-export` for hash-verified raw-frame decoding. | No provider sequence; local receive sequence only. Synchronous raw-frame journaling is not load-certified. Current path must not claim Full D30 or full CAS modelling. Rust build/live account validation remain owner tasks. |
| FYERS | `apps/fyers-recorder` | Data Socket SDK callback capture for `SymbolUpdate` and optional five-level `DepthUpdate`, hash-chained journal and explicit local gap records. QuantLab normalizes callbacks to canonical event observations. | Published Standard/Prime Data Socket ceilings are 100/5,000 symbols per user. `--max-symbols` is operator-declared (default 100), not remote entitlement verification. The separate 50-level Protobuf TBT socket is not implemented. Callback JSON is not original wire data; no exchange sequence. Live handshake untested. |
| Zerodha Kite | `apps/kite-ops` | Read-only profile/funds/orders/positions and instrument-master checks; constrained manual CNC BUY LIMIT DAY operator workflow. | Not a production OMS. Sample/private policy keeps live order flag false. Static-IP, daily-auth and market-session attestations are operator inputs, not remotely verified facts. Live account/order behavior not tested. |

## Build and test commands

```bash
python3 -m pip install -e 'research/python[dev]' -e 'apps/fyers-recorder[test]' -e 'apps/kite-ops[test]'
make python-test
python3 scripts/check_repo.py check
cargo build --release -p upstox-tools --bins --locked
cargo test -p upstox-feed-proto --locked
```

For FYERS Prime Data Socket, pass `--max-symbols 5000` only after confirming the account's plan. FYERS TBT subscription ceilings are separate from Data Socket limits; the current recorder does not implement TBT. For Upstox, check the subscribed mode and combined per-user limits before changing watchlists. Official docs are linked in `engineering/provider-boundaries.md`.

## Live order rule

Kite's limited operator surface is separate from the reference OMS and requires an operator-private policy. The independent signed live permit verifier is research/reference infrastructure only; its successful verification does not activate routing. Unknown broker outcomes are never blindly retried. The current OMS always rejects live submissions. Do not bypass this condition with flags or fixture keys.
