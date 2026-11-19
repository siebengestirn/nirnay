# SBE GAP-2 capability inventory — 2026-11-19

**Scope:** `extras/` SBE decoder, schemas and golden corpus only. This is a
capability inventory and dependency review with a promotion map, **not** a
promotion and **not** a compatibility claim. No `extras/` file is copied or
modified by this record.

## 1. Why this is GAP-2

`engineering/extras-audit.md` retains `extras/cpp` (SBE decoder + conformance
probes), `extras/schemas/challenge`, `extras/tests/golden` and `extras/rust/`
under `extras/`, excluded from the root workspace. The Rust drivers are
explicitly not wired in: they import predecessor types that no longer exist in
the trimmed `extras/src` (`util` now exposes only `sha256_hex` and `hex_lower`).
Closing GAP-2 means giving each retained obligation an active Rust home and an
independent differential test. `extras/` retirement stays blocked until then.

## 2. Inventory

### 2.1 Schemas and message definitions

- `extras/schemas/challenge/challenge_orderbook.yaml`: schema id 1, version 2.
  `Snapshot` template 1 carries received/exchange timestamps, bids/asks groups
  of fixed-point `price_level` and `quantity` int64, and a `symbol` string.
  `PriceLevelUpdate` template 2 carries two timestamps, `price_level`,
  `quantity`, `SideEncoding` and `symbol`. Field ids are part of the wire
  contract.
- `extras/schemas/challenge/common-types.xml`: predecessor type dependencies —
  `messageHeader` (8 bytes: blockLength/templateId/schemaId/version uint16),
  `groupSizeEncoding` (blockLength/numInGroup uint16), `varStringEncoding`
  (uint32 length plus UTF-8 varData), `UTCTimestampMicrosEncoding` and its
  nullable variant, `bytes32`, `address`, `SideEncoding` (uint8, 0=buy 1=sell)
  and `BooleanType`. Changing an encoding changes every schema using it.
- `extras/schemas/internal/v1.yaml`: a **separate** internal Rust-to-C++
  envelope contract (event/instrument identity, shard/replica, seq/clock
  presence, receive/exchange time, continuity/reason). It is not the challenge
  SBE schema and is not decoded by `extras/cpp/src/sbe.cpp`.

### 2.2 Golden captures and provenance

- `extras/tests/golden/sample_replay.sbe` and
  `baseline_pre_types_book.sbe` are byte-identical: 4,096 bytes, SHA-256
  `af291b403553b6e0d182f8001643e03a055e4d7e8b0dbbdfee54c3ad56f50e14`.
- Provenance: frozen before the types/book extraction from source commit
  `799b6b58545799fac387e1feb08a188cba0d6f44`, fixture
  `tests/fixtures/sample_replay.jsonl`. The metadata forbids regenerating the
  file to make a failing comparison pass.
- Content: ten **synthetic** Kalshi `TEST-A`/`TEST-B` order-book messages
  (snapshots and price-level updates). No real market data and no account data.
- The independent C++ canonical dump (`eng_sbe_dump <golden> 4096`) is ten lines
  with SHA-256 `cba8bae7984408893e34a734b3db5e8598446932181504ffd04b7dca1ae9eaed`.

### 2.3 Decoder outputs and error semantics

- `extras/cpp/src/sbe.h` and `sbe.cpp` define `decode_message`,
  `encode_message`, `decode_capture` (chunk size >= 1024) and the canonical
  `SbeMessage` (template id, two timestamps, symbol, bids/asks levels, optional
  update fields).
- Fail-closed errors: truncated u16/u32/u64/block/group/var-data, unknown
  schema id or version, unknown template id, snapshot block length != 16,
  update block length != 33, invalid group entry length, `SideEncoding > 1`,
  symbol length above u32, group size above u16, a message tearing across an
  aligned chunk, and chunk size < 1024. A zero header at a chunk end is padding
  that terminates the chunk.
- Canonical dump format: one `snapshot sym:recv:exch:bids=…:asks=…` or
  `update sym:recv:exch:side=n:price=n:qty=n` line per message, byte-for-byte
  diffable between independent decoders.

### 2.4 CTest targets and conformance assertions

- Registered CTest: **one** — `sbe_roundtrip` (`eng_sbe_test` over
  `tests/golden/sample_replay.sbe`, chunk 4096). `eng_conform` (order-book line
  driver) and `eng_sbe_dump` (SBE dump) are built but not registered as CTest.
- The SBE test asserts ten messages, first snapshot, decoded symbol, bid price
  400000, third message an update with buy side; per-message encode/decode
  round-trip and consumed length; and seven malformed-input throw cases
  (truncated header, unknown template, invalid side, unknown schema, bad group,
  chunk < 1024, cross-chunk tear).
- `scripts/conformance.py --require-sbe` is a separate gate requiring both SBE
  dump drivers and byte-identical output, with no skipped-comparison success
  path. The checked-in historical report records `gate_passed: false` and must
  not be cited as verification.

### 2.5 Fixtures and licence

- `tests/fixtures/conform_basic.txt` (14 order-book operations),
  `tests/fixtures/numeric_cases.jsonl` (8 fixed-point cases) and
  `tests/fixtures/sample_replay.jsonl` (16 source records for the golden).
- `extras/LICENSE` is MIT, Copyright (c) 2026 Kartik Jangid. It must travel with
  any copy or promotion. Fixtures are synthetic; no captured provider payloads
  are restored.
- The complete pre-cleanup snapshot is external at
  `../legacy-snapshot-2026-11-13/legacy-full-backup-2026-11-13.tar.gz`
  (SHA-256 `a49e864308a7e58d667a86049dd80e8ff2c631cd762e50a74aff60007464f9b1`).

### 2.6 Predecessor type dependencies — the blocker

- `extras/rust/sbe_dump.rs` imports
  `nirnay_extras::util::{AlignedSbeReader, OrderbookSchema,
  dtos::challenge_orderbook::{AnyMessage, PriceLevelUpdateDomain, SnapshotDomain}}`
  plus predecessor `OrderBook`/`Price`/`Quantity` types. None exist in the
  trimmed `extras/src` (`lib.rs` exports only `util` and `venue`).
- `extras/rust/conform_driver.rs` imports `nirnay_extras::{Side,
  model::{Level, Price, Quantity, QuantityChange, QuantityDelta},
  orderbook::OrderBook}` — also absent from the trimmed crate.
- The retained Rust drivers therefore cannot compile as-is; they are reference
  material, not an active implementation. No crate in the root workspace
  decodes SBE: `schemas/` holds the Upstox protobuf and research/execution JSON
  schemas only.

## 3. Promotion map: obligation → active home → independent differential test

| Obligation | Active home | Independent differential test |
|---|---|---|
| Challenge schema v1/v2, templates 1/2 | `crates/sbe-codec` schema constants and decoder | decode the frozen golden; assert ten messages and the C++ dump's kinds, timestamps and symbols |
| `common-types.xml` predecessor encodings | decoder primitives in `crates/sbe-codec`, with no dependency on removed predecessor types | block-length and `SideEncoding` assertions mirroring the seven C++ malformed cases |
| Golden corpus and provenance | `crates/sbe-codec/tests/golden/sample_replay.sbe` copied verbatim with `LICENSE`, `.meta` and `.sha256` | assert digest `af291b…50e14` and 4,096-byte length before decoding |
| `decode_message` / `encode_message` / `decode_capture` | typed errors in `crates/sbe-codec` | per-message round-trip; canonical dump compared byte-for-byte with the captured independent C++ dump (`cba8bae7…9eaed`) |
| Error semantics | typed `SbeError` variants | seven malformed cases reproduced independently in Rust, each failing closed |
| CTest `sbe_roundtrip` | Rust integration test in `crates/sbe-codec` | an independent test, not a port of the C++ test; the C++ CTest stays the reference gate in `extras/` |
| Fixtures and licence | golden plus MIT licence copied with the promotion path recorded in `engineering/migration-map.csv` | licence/provenance hashes checked; no provider payloads |
| Conformance assertions | dump-equality assertion in the Rust test | explicitly excludes the numeric/book oracle; no SBE compatibility claim from book conformance |
| `--require-sbe` gate | Rust dump driver plus the committed C++ dump golden | byte equality with frozen independent output; missing driver or mismatch fails closed |

## 4. Boundaries

- Inventory only: no `extras/` edit, no file copy, no new crate created here.
- No SBE compatibility claim from the C++ numeric/book oracle; SBE is a separate
  protocol obligation.
- No live path, broker, trading, source acquisition or valuation.
- `extras/` retirement remains blocked until every obligation has an active home
  and an independent test, and the owner approves the re-audit.
- This record promotes no capability or claim state.
