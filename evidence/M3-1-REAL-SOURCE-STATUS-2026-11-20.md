# M3.1 real-source status — 2026-11-20

## Verified engineering result

The five-level NSE cash pipeline is implemented end to end and verified with
synthetic conformance fixtures:

- source capability and entitlement registry (`configs/providers/source-products.v1.json`);
- scope-aware continuity with fail-closed reconnect and `Unverified` state;
- typed five-level MBP snapshots from Upstox V3 Full and FYERS Data Socket
  callbacks, with explicit price/time conversion and no MBO claim;
- one depth-bounded MBP book per source with evidenced snapshot recovery;
- kernel envelope carrying typed payloads, with the paper runner and risk
  authority consuming kernel-derived book state (parallel quote path removed);
- cross-source reconciliation that reports differences and never merges books;
- deterministic fresh-process replay from a verified session into a digest-bound
  report.

Local gates: `make check` exit 0; workspace tests, clippy `-D warnings`, 21 JSON
schemas, 175 Python tests and 2 subtests all pass. Each PR had exact-head CI
12/12 and independent review closure.

## Real Upstox session — captured and replayed

With the owner's explicit read-only authorization, a bounded 90-second live
Upstox V3 Full capture was taken on 2026-11-20 from the keychain analytics
token. Credential values were never printed or exported; all artifacts are
outside Git under `.local/live-sessions/2026-11-20/upstox/`.

- Instrument: `NSE_EQ|INE848E01016` (NHPC), resolved to `IN:INE848E01016`
  against the dated NSE cash master.
- Manifest: 37 frames, 0 decode errors, 1 connection, capture-local sequence;
  `provider_sequence_available: false`.
- Raw journal SHA-256:
  `762b87c07e6b720980850c53afa233ae1852be373c16d3c6971bf641e9f2f143`.
- Decoded offline into 36 five-level Full-feed observations and projected by
  `upstox-kernel-session` into a verified kernel session (SHA-256
  `fda9291a240ef226fd374a73d74fc265de7b3bd1868fd782bfe331c69523a1aa`).
- `session-replay` rebuilt one live five-level MBP book: best bid 7,644 paise,
  best ask 7,648 paise, 5 bid and 5 ask levels. Report SHA-256
  `6dc63204f2862b146ab36cb54e5873f5bd89a537471d7590361f9f80690acc0a`;
  fresh-process replay is byte-identical.

Continuity is capture-local receive order, not exchange packet completeness.
No order was submitted and no broker write occurred.

## Real-source certification: PARTIAL

Upstox five-level capture, decode, identity, book reconstruction and replay are
demonstrated. FYERS remains blocked: the keychain holds only app ID and secret,
not the required Data Socket access token, so no FYERS live capture was run.
Cross-provider reconciliation against two real sessions and the remaining
twelve acceptance conditions are therefore **not claimed**.

## Required owner input

A FYERS Data Socket access token (or an entitled captured FYERS session),
plus a session covering open, reconnect and close phases, before full M3.1
real-source certification.

No live order submission, broker write, D30, FYERS TBT, MBO, BSE, MCX or
derivatives work was performed.
