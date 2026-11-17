# Point-in-time market data and corporate-action lineage

**Status:** implemented research pipeline; source coverage and entitlements remain external inputs.  
**Owner:** research-data / market-data.  
**Applies to:** canonical observations, instrument listings, historical universes, corporate actions and immutable dataset releases.

## 1. Required time axes

Nirnay does not collapse all timestamps into one `timestamp`:

- **Provider observation time:** the source's timestamp field, with the exact source field and unit retained. It is not assumed to be a matching-engine timestamp.
- **Last-trade time:** a distinct per-instrument last trade timestamp where the provider supplies one. A last-trade snapshot is not a unique trade event.
- **Local receive time:** when this process received the payload. It is the `known_at` time for this capture. A reference record retrieved later cannot be retroactively treated as known at this time.
- **Local monotonic receive time:** process-local duration/order evidence. It must not be compared across hosts or process restarts.
- **Reference valid time:** dates for which a listing or membership applies.
- **Reference knowledge time:** when Nirnay recorded the reference record and was able to use it.

The dataset builder chooses session date from provider source time, then last-trade time, then local receive-time fallback. The selected method is written on every row and in the manifest. The fallback adds a warning. For Upstox, `currentTs` is documented as the timestamp of the received tick; it is not represented as a matching-engine timestamp. See the provider-specific semantics in `provider-normalization.md`.

## 2. Raw-to-derived chain

The expected evidence chain is:

1. **Upstox:** original binary Protobuf frame in `raw-frames.jsonl`, with payload SHA-256 and capture metadata. Run the active Rust `upstox-decode-export` tool to produce full decoded message envelopes. Do not use `decoded-observations.jsonl` as a market-data dataset: it is a diagnostic projection and omits full depth/price fields.
2. **FYERS:** SDK callback journal, with local receive sequence, a SHA-256 hash chain and explicit callback-gap records. This is decoded SDK callback JSON, not original WebSocket wire bytes.
3. **Canonical event:** `MarketEvent` with provider, provider instrument key, canonical instrument ID (when mapped), venue, exact decimal strings, timestamp semantics, raw payload hash, optional source-frame/callback-record hash and quality flags.
4. **Point-in-time row:** canonical event plus resolved listing revision, bitemporal universe membership (when a universe filter is requested), known corporate-action/revision lineage and exact split factor.
5. **Dataset publication:** immutable `events.jsonl`, optional Zstandard Parquet and a manifest with hash sets for every lineage class.

The SHA-256 fields establish internal linkage and corruption detection. They do not prove that a broker supplied every event, that a source feed was lossless, that the source was licensed for the use, or that the source itself was truthful.

## 3. Instrument/listing identity

Provider instrument keys are not canonical instruments. Every provider key must map through a bitemporal listing row with:

- stable listing ID and canonical instrument ID;
- provider name and provider instrument key;
- venue;
- valid-from / valid-to dates;
- recorded-at nanosecond timestamp;
- source ID and source SHA-256;
- optional `supersedes_listing_id` for a later correction.

Resolution uses the event's observation date and local receive time as knowledge time. Missing and ambiguous mappings fail closed. The active code does not infer corporate identity from ticker strings or silently map an unknown provider key.

## 4. Historical universe / survivorship

For a universe-filtered dataset, supply explicit bitemporal `universe_memberships` rows. A membership has valid dates, recorded-at time, source hash, state and optional supersession. The builder includes a row only when that membership was valid on the observation date and known by that event's receive time.

The manifest records the membership source-hash set but always sets `survivorship_completeness_independently_verified=false`. This reflects a source limitation: Nirnay cannot prove that an upstream universe file includes delisted, suspended, merged or renamed instruments. A surviving-constituent-only source must not be labelled survivorship-free.

## 5. Corporate actions

Corporate actions are immutable records; a correction creates a new revision that supersedes its predecessor. The record includes action ID, instrument, kind, announced time, source retrieval/recorded time, effective/ex-date, optional record/payable dates, exact integer share ratio, cash amount, source ID/hash and revision status.

- Split and bonus adjustments are exact `Fraction` views. The raw price in the canonical event is never overwritten.
- Dividend actions remain cash-flow records. The current single-instrument simulator does **not** yet book dividend receivables/payments, withholding, tax, or reinvestment; the dataset lineage does not equal a total-return series.
- Rights, mergers, demergers, delistings, symbol changes and unrecognised actions fail closed until an independently approved point-in-time transformation exists. No generic ratio is fabricated.
- Revised/cancelled actions preserve the entire known revision chain. The version visible at each observation's knowledge time is used.

## 6. Dataset commands

Install the package in editable mode:

```bash
python3 -m pip install -e 'research/python[dev,research]'
```

Normalize provider streams only after creating canonical instrument maps:

```bash
quantlab normalize --provider upstox \
  --input data/decoded/upstox-full-feed.jsonl \
  --instrument-map configs/providers/canonical-instrument-map.json \
  --venue-map configs/providers/canonical-venue-map.json \
  --output data/normalized/upstox.jsonl

quantlab normalize --provider fyers \
  --input data/market-sessions/fyers/market-data.jsonl \
  --instrument-map configs/providers/fyers-to-canonical.json \
  --output data/normalized/fyers.jsonl
```

Prepare a reference-data JSON file with `listings` and `corporate_actions` arrays; optionally include `universe_memberships`. The loader expects ISO `YYYY-MM-DD` dates and nanosecond integer times:

```bash
quantlab build-dataset \
  --events data/normalized/upstox.jsonl \
  --reference-data data/reference/pit-reference-data.json \
  --dataset-id nse-cash-2026-11-12-v1 \
  --created-at-utc-ns 1794500000000000000 \
  --output data/datasets/nse-cash-2026-11-12-v1

quantlab verify-dataset data/datasets/nse-cash-2026-11-12-v1
```

Add `--parquet` for a Zstandard Parquet companion (requires `pyarrow`). To sign the manifest using AWS KMS, add `--kms-key-id arn:aws:kms:REGION:ACCOUNT:key/KEY_ID --kms-region REGION`; install the `kms` optional extra and use a workload identity with narrowly scoped KMS permissions. Verify a signed dataset using the trusted public key distributed independently of the dataset:

```bash
quantlab verify-dataset data/datasets/nse-cash-2026-11-12-v1 \
  --trusted-key dataset-release-key-2026=BASE64_ED25519_PUBLIC_KEY
```

The output directory must not already exist. A dataset is never overwritten; publish a new dataset ID/path for a correction. The publisher stages files, fsyncs them and atomically renames the completed sibling directory. `verify-dataset` verifies internal hash/manifest consistency, not the upstream source. Optional `--parquet` writes Zstandard-compressed Parquet and verifies semantic row equivalence against the canonical rows. Optional `--kms-key-id` uses a customer-managed AWS KMS Ed25519 key to sign a domain-separated SHA-256 digest of the canonical manifest; verification requires an independently distributed `--trusted-key KEY_ID=BASE64` public key. The detached signature proves control of the publishing key and integrity of the manifest digest; it does not prove the upstream market/reference source is authentic or complete.

For multi-provider rows, keep provider streams separate for data quality review. A downstream research dataset may contain both providers for comparison, but an execution simulation must select one observation stream per instrument/time; it must never add Upstox and FYERS visible quantity together.

## 7. Admission requirements for production datasets

Before a dataset may be considered suitable for any future production decision:

- canonical instrument map coverage and duplicate/conflict review;
- delisted/suspended/inactive listing completeness from an authoritative source;
- corporate-action coverage and correction/cancellation checks against exchange or licensed reference data;
- data-rights/retention/derived-data permission confirmation;
- session calendar and auction/pre-open handling;
- clock quality, feed-gap and stale-data reports;
- event-count and duplicate reconciliation against independent provider statements;
- restore/rebuild verification from original raw archives;
- optional detached Ed25519 dataset-manifest signature checked against an independently distributed public-key trust registry (implemented); and
- external authorization of the publisher key, source authenticity/completeness and entitlement evidence (not established by this code).

Until those checks are complete, even a cryptographically signed manifest is only an authenticated publication of a reproducible research artifact, not an exchange-certified historical tape.
