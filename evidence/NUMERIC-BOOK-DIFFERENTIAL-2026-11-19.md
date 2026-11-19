# Numeric and book byte-level differential — 2026-11-19

**Scope:** closes the P0 numeric/book differential blocker from the owner
directive. A shared versioned corpus is fed byte-for-byte to the C++ oracle
drivers and their Rust counterparts; stdout and exit-status class are compared
exactly. No tolerance is used where the contracts specify exact equality. No
live path, broker or trading component is involved.

## What was added

- `verification/cpp-oracle/src/book_driver.cpp` and `numeric_driver.cpp`:
  verification-only C++ drivers built by the oracle CMake target.
- `crates/market-book/src/bin/book_driver.rs` and
  `crates/market-types/src/bin/numeric_driver.rs`: active Rust counterparts.
- `verification/corpus/` version 1: 7 book cases and 4 numeric cases. Book cases
  cover ordering, best-first output, delta application, replay, zero removal,
  overflow rejection, negative-result rejection and unknown operations. Numeric
  cases cover parse, format, rescale, boundaries and invalid scales/input.
- `scripts/check_numeric_differential.py`: fail-closed gate that takes explicit
  driver paths, hashes each input and compared output, and writes a JSON report.
- A new `numeric-differential` CI job builds both sides and runs the gate on
  Ubuntu with GCC and the pinned Rust toolchain.

## Defects found by the differential

The first run failed 2 of 11 cases, which is the purpose of the gate:

1. `numeric/invalid.txt`: the C++ driver consumed the operation token before a
   numeric exception and emitted a truncated result line. Both drivers now
   compute the full result inside the `try` block before writing a line.
2. `numeric/rescale.txt` and `numeric/format.txt`: Rust `format_scaled` silently
   fell back to `value.to_string()` for an unsupported scale while C++ rejected
   it. The Rust driver now validates the scale before formatting and emits the
   same `error format` outcome. The underlying library fallback remains
   unchanged here; this record does not claim it was removed.

## Final verification

All 11 cases pass with identical stdout bytes and exit-status class:

| Group | Cases |
|---|---|
| Book | `basic`, `ordering`, `boundaries`, `replay`, `invalid_overflow`, `invalid_negative`, `invalid_op` |
| Numeric | `parse`, `format`, `rescale`, `invalid` |

`make check` PASS; source-style and Rust-style PASS; cross-pollution check PASS.
Hosted CI evidence is recorded by the PR and exact-head run.

## Boundaries

The corpus is synthetic; no provider, venue or account data. The C++ target is a
verification oracle, not a runtime component. Byte equality is asserted for
these exact cases on the pinned toolchain; the record makes no broader claim
about all fixed-point inputs, all book states or production readiness.
