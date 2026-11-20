# Provider data-rights registry correction — 2026-11-20

Dated evidence for the permission-model and product-semantics corrections.
Immutable record.

## Defects

1. `configs/providers/source-products.v1.json` recorded Upstox and FYERS
   `rights.store: false`, `rights.replay: false`, `entitlement.verified: false`
   as plain booleans, while owner-local activity had in fact stored, exported
   and replayed the captured Upstox session. A boolean could not distinguish
   "not verified" from "prohibited" and carried no evidence provenance.
2. The Kite quote product declared `snapshot: false` but
   `incremental_updates: snapshot_only`; its quote API returns a market-depth
   snapshot.

No conclusion is drawn that the prior activity breached any agreement. The
defect was that the registry and the activity did not agree, and the registry
could not express the distinction.

## Correction

- Rights are now tri-state objects: `UNKNOWN`, `PERMITTED`, `PROHIBITED`, each
  with `source`, `document_reference`, `scope` and `review_date`. `UNKNOWN`
  never grants a right. No permission was silently upgraded to `PERMITTED`:
  all four rights for all three products are `UNKNOWN` with source
  `owner_reported`, pending written evidence.
- Added registry-level `rights_review` with status
  `pending_written_evidence` and the seven required evidence classes: personal
  capture, local retention, historical replay, backup retention, internal
  analytics, external redistribution and retention period.
- Corrected Kite quote to `snapshot: true` while retaining
  `incremental_updates: snapshot_only` and `source_sequence: unavailable`.
- Added cross-field schema rules: `snapshot_only` implies `snapshot: true`;
  `fresh_snapshot` recovery implies `snapshot: true`; `mbo` depth implies
  `order_level_identity: true`.
- `crates/market-archive` parses the new model; store/replay requirements fail
  unless the right is explicitly `PERMITTED`. Tests cover UNKNOWN fail-closed,
  the written-evidence checklist and the corrected Kite semantics.

## Boundary

This correction records verification state only. It does not grant data rights,
does not certify entitlements, and does not authorize Monday capture before the
owner supplies the written evidence. No provider credential, broker write or
live order is involved.
