# Upstox / FYERS normalization and cross-feed reconciliation

**Status:** implemented canonical adapters with synthetic/fake fixtures; no live provider certification.  
**Canonical model:** `research/python/src/quantlab/market.py`.  
**Adapters:** `providers.py`.  
**Reconciliation:** `reconcile.py`.

## 1. Provider semantics are not interchangeable

### Upstox Market Data Feed V3

The official V3 interface is a WebSocket carrying Protobuf-encoded data. Nirnay captures the original binary payload before decoding and keeps a local receive-order sequence; it does not invent an exchange sequence. Current official subscription limits are mode- and plan-dependent: Normal limits document 2 connections/user; LTPC 5,000 individual / 2,000 combined instrument keys; Option Greeks 3,000 individual / 2,000 combined; Full 2,000 individual / 1,500 combined. Upstox Plus documents Full D30 at 50 individual / 1,500 combined. Standard Full includes five depth levels; Plus Full D30 includes 30. Do not infer the account's actual entitlement from those published ceilings. The active recorder/decoder path must not claim D30 support unless it explicitly implements and verifies that mode. The current feed also exposes `market_info` and CAS-related fields (including indicative equilibrium price during auction windows); these must not be interpreted as executed trades. The active canonical event model does not yet encode the complete auction state, so auction intervals are excluded from executable simulation unless explicitly handled.

The normalizer preserves:

- `currentTs`: timestamp of the received tick (feed envelope), not a matching-engine timestamp;
- `ltpc.ltt`: per-instrument last-traded-time field;
- `ltpc.ltp` and `ltpc.ltq`: current last-traded price/quantity fields;
- full-feed `marketLevel.bidAskQuote[]`: displayed bid/ask levels when present;
- provider instrument key and original payload/frame hashes.

The Rust `upstox-decode-export` binary converts verified raw frame journal entries into full decoded JSON envelopes. The older `decoded-observations.jsonl` file is only a diagnostic projection and is rejected as a canonical full-depth source.

Official API: <https://upstox.com/developer/api-documentation/v3/get-market-data-feed/>.

### FYERS Data Socket V3

The recorder stores callback JSON emitted by the official SDK. `SymbolUpdate` (`sf`) and `DepthUpdate` (`dp`) have different shapes. SymbolUpdate carries current-day price/volume fields and last-traded time; DepthUpdate is a displayed depth snapshot with five levels in the Data Socket interface. Separately, FYERS documents a Tick-by-Tick (TBT) socket for 50-level NSE equity/NFO depth using Protobuf. TBT is not captured or decoded by the current Python recorder. Published Data Socket limits are Standard 100 symbols/user and Prime 5,000; TBT has separate limits of Standard 1 and Prime 15 (up to three connections of five). `--max-symbols` is an operator-declared entitlement bound, defaulting to 100, not an automated tier detector. The SDK callback archive is not a raw WebSocket packet capture, and the documented callback contract does not make its local sequence an exchange sequence.

Official docs:

- <https://fyers.in/api-docs/v3/api/websockets/data-socket/data-socket-symbol-update/>
- <https://fyers.in/api-docs/v3/api/websockets/data-socket/data-socket-depth-update/>
- <https://fyers.in/api-docs/v3/api/websockets/data-socket/>
- WebSocket overview / Data Socket and TBT distinctions and current published caps: <https://fyers.in/api-docs/v3/api/websockets/websocket-overview/>

## 2. Canonical event contract

A canonical event carries `provider`, `provider_instrument_id`, optional canonical `instrument_id`, venue, event kind, local receive timestamp, optional local monotonic timestamp and sequence, source timestamp plus semantics, last-trade time plus semantics, exact `Decimal` prices, quantities, depth levels, raw payload hash, optional original frame/callback-record hash and quality flags.

Allowed kinds are `quote`, `depth`, `trade`, `last_trade_observation`, `index` and `status`.

**A changing LTP snapshot is not emitted as a confirmed trade print.** Upstox/FYERS normalizers
  emit `last_trade_observation` only when the last-trade tuple advances. The event explicitly
  labels this as an observation, with no aggressor side. It cannot consume a passive simulated
  queue. Only a source that provides unique trade events can emit `event_kind=trade` and supply
  the fields needed for passive queue modeling.

## 3. Identity maps

Provider symbols/keys must be mapped to a canonical instrument ID via a reviewed mapping file. The canonical listing table is separately bitemporal and binds provider key + provider + venue to the canonical instrument for valid and knowledge intervals. Do not merge on ticker alone, and do not infer a symbol mapping from similar text.

The optional venue mapping file resolves provider segment prefixes to the canonical venue label. A venue mismatch is a hard reconciliation status, not a soft price disagreement.

## 4. Reconciliation

`quantlab reconcile` selects each provider's latest comparable quote/depth state for a canonical instrument. It reports:

- missing provider;
- stale observations / future receive timestamps;
- receive-time skew;
- source-time incomparability or excessive source skew;
- crossed/invalid books;
- comparable price basis and basis-point difference;
- within-tolerance consistency or price divergence.

It compares two-sided midpoint to two-sided midpoint when both feeds provide it, otherwise last-traded price when both provide comparable last prices. It does not average prices, merge depth, sum sizes, or choose an execution route. A reconciliation result is a quality signal; it is not a consolidated exchange feed.

### Time and quality policy

- Keep provider clocks and local capture clocks in separate fields.
- No cross-host `received_monotonic_ns` comparisons.
- Do not use `currentTs` as an exchange sequencing guarantee.
- Do not create missing quotes by carry-forward and then call them fresh. Freshness is evaluated at reconciliation time.
- Preserve crossed books, gaps and regressions as quality defects. Do not sort away or silently smooth continuity errors.
- For FYERS, the normalizer verifies the recorder hash chain when present and rejects mixed chained/unchained rows. Explicit gap records block publication unless `--allow-gaps` is used; output then remains non-research-grade.

## 5. Feed selection in simulation

Upstox and FYERS observe the same underlying NSE market. Their quantities are two views of the same liquidity, not two independent venues. The event simulator requires a single selected provider when the replay contains both. A future consolidated state model must define conflict precedence and not sum depth. Current real callback data alone does not provide the aggressor-tagged prints required for queue consumption.
