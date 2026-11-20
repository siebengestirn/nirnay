# SBE GAP-2 promotion evidence — 2026-11-19

**Code commit:** `1b737784a7897d4647add01292676e6fb8351adc`.
**Scope:** promotion of the challenge SBE decoder and golden corpus into an
active, self-contained Rust crate. No `.local/extras/` edit, no live path, no broker or
trading integration.

## What was promoted

- `.local/extras/cpp/src/sbe.cpp` → `crates/sbe-codec/src/decode.rs`: schema id 1,
  version 2, `Snapshot` template 1 and `PriceLevelUpdate` template 2, with
  `decode_message`, `encode_message`, `decode_capture`, a canonical dump
  formatter and typed `SbeError` variants. No predecessor monolith types.
- `.local/extras/tests/golden/sample_replay.sbe` → `crates/sbe-codec/tests/golden/`,
  copied verbatim with its `.sha256` sidecar, `.meta` provenance and the MIT
  `LICENSE`.
- The independent C++ canonical dump is frozen as
  `tests/golden/cpp_canonical_dump.txt` with its own SHA-256 sidecar. It was
  produced by `eng_sbe_dump <golden> 4096` from the C++ SBE decoder retained
  under `.local/extras/cpp`; it is a **protocol** oracle only.

## Inventory closure

| Obligation from `evidence/SBE-GAP-2-INVENTORY-2026-11-19.md` | Active home |
|---|---|
| Challenge schema v1/v2, templates 1/2 | `crates/sbe-codec/src/decode.rs` |
| `common-types.xml` encodings | decoder primitives; no removed predecessor types |
| Golden corpus and provenance | `tests/golden/` with licence and hashes |
| Decoder outputs and error semantics | `SbeError` variants and canonical dump formatter |
| CTest `sbe_roundtrip` | `tests/differential.rs` (independent test, not a port) |
| Fixtures and licence | golden plus MIT licence; migration-map rows recorded |
| Conformance assertions | byte-equality against the C++ SBE dump only |
| `--require-sbe` gate | frozen C++ dump compared byte-for-byte with Rust output |
| Predecessor type dependency | removed: the active crate has no `nirnay_extras` dependency |

## Tests

`crates/sbe-codec/tests/differential.rs`:

- `golden_capture_matches_frozen_provenance`: 4,096-byte golden, digest
  `af291b403553b6e0d182f8001643e03a055e4d7e8b0dbbdfee54c3ad56f50e14`.
- `rust_dump_matches_the_independent_cpp_sbe_dump`: ten lines, digest
  `cba8bae7984408893e34a734b3db5e8598446932181504ffd04b7dca1ae9eaed`, byte
  equality with the captured C++ output.
- `canonical_reencoding_is_byte_identical_to_the_golden`: ten contiguous
  messages ending at byte 732, each re-encoded to its exact original bytes.
- `seven_malformed_cases_fail_closed`: truncated header, unknown template,
  invalid `SideEncoding`, unknown schema id, unknown schema version, invalid
  group block, chunk below minimum and a genuine cross-chunk tear. The tear is
  constructed from a 4,086-byte filler message so the loop reaches the boundary
  directly; the test asserts the exact `MessageTear` error.
- `invalid_utf8_symbol_fails_closed`: a non-UTF-8 symbol byte is refused.

## Verification

`cargo test -p sbe-codec` — 5 passed. `make check` PASS with full workspace and
175 Python tests. Capability CAP-026 and claim CLM-020 are `implemented`, not
`verified`.

## Boundaries

The differential oracle is the C++ **SBE** decoder, not the C++ numeric/book
oracle; no compatibility is claimed for book or numeric conformance. The golden
is ten synthetic Kalshi `TEST-A`/`TEST-B` messages, not live market data.

Rust `read_string` enforces UTF-8 while the C++ decoder copies raw symbol bytes.
All corpus symbols are ASCII, so byte-for-byte equivalence is established for
this frozen corpus, not for arbitrary non-ASCII symbol bytes; a non-UTF-8 symbol
is refused by the active decoder.

`.local/extras/` retirement remains blocked until a fresh audit and owner approval.
## Later amendment (2026-11-20)

The synthetic golden was subsequently re-symbolized to `nirnay:TEST-A` and
`nirnay:TEST-B` (same 6-byte symbol field) to remove predecessor venue names
from the active workspace. The current golden SHA-256 is
`022cb71b7f2c7d9e5200a60096b3ffcd39dbe8c03bd69cd1c92a196d6aac6590` and the
current independent C++ canonical dump SHA-256 is
`aa555aae39f9cd30f911e533d1a2227f2d6a14425c906b472be7473c3b2a2a5c`.
The digests above remain the historical values recorded at promotion time.
