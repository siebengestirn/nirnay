# Real-source conformance record — 2026-11-18

**Prepared:** 2026-11-18 (operator-supplied dated exchange files).
**Revision:** `0a9e48659e00ae2e1baa6f224db8c044c5deede7` (code commit for the
retained transcript; the first conformance run was at `2ebacd7c…`, and the only
later code changes were a justification comment and an added synthetic test
row, with no parser behavior change). This record is added in a separate
evidence commit.
**Environment:** macOS 27.0.1 arm64; rustc/cargo 1.99.1.
**Scope:** compatibility of the two declared source versions and categories
listed below only. This is local engineering evidence, not a production,
market-data entitlement, exchange approval or profitability certification.

## Gate executed from the committed tree

```bash
NIRNAY_SOURCE_FIXTURES_DIR=/Users/tourist/Documents/nirnay-source-fixtures/2026-11-18 \
  cargo test -p india-reference --test source_conformance --offline -- --ignored --nocapture
```

Exit status: **0**. Verbatim result lines:

```text
test dated_exchange_sources_conform ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.68s
```

The complete raw transcript is retained at
`evidence/source-conformance-2026-11-18-output.log` (413 bytes). Its SHA-256 is
`7299236bc788d81b0e3211c3e54b853dc03593f5c188e33b30daffdd83a7f9ed`. That hash
identifies the retained transcript bytes; a fresh rerun will differ because
Cargo embeds wall-clock timings, so it is not presented as a reproducible
identifier of any future run.

The gate verifies each fixture's exact byte size and SHA-256 before parsing,
declares an exact per-venue scope, asserts every physical row has exactly one
outcome, asserts declared accepted/excluded/rejected counts, and repeats both
parses to prove byte-identical outcomes. An undeclared source version is
separately asserted to fail closed.

## Source files and accounting

| Source | File | Format version | Coverage | Physical | Accepted | Excluded | Rejected | SHA-256 |
|---|---|---|---|---:|---:|---:|---:|---|
| NSE | `nse-equity-l.csv` | `nse-cash-equity-v1` | full:NSE | 2,632 | 2,632 | 0 | 0 | `a6b184ea3e372eca6c2322bd252e66911b0648145dc63181469c73fa7a0687cc` |
| BSE | `bse-eq-scrip-18112026.csv` | `bse-security-master-v2` | full:BSE | 20,954 | 4,856 | 14,808 | 1,290 | `9f2c134f6409f502089416d56b08eb195dc0306b467764278780544f5b80a5f0` |

Fixture manifest SHA-256:
`d37d388ff2a81bf258db6b2b54e6eadeecf35625bb54c48f97c9cf3287b53eaa`
(`provenance.json`; records source URL, retrieval time, declared format and
coverage, and the expected counts above).

NSE source URL: `https://nsearchives.nseindia.com/content/equities/EQUITY_L.csv`,
retrieved `2026-11-18T21:23:48Z`, 185,917 bytes.

BSE source: operator-supplied `scrip.zip`, member file
`SCRIP/BSE_EQ_SCRIP_18112026.csv` (dated 18-11-2026 in filename and archive
entry), retrieved from the operator `2026-11-18T21:38:03Z`, 4,442,888 bytes. No
public direct URL for this member-distribution file was established; the
operator-supplied archive is the provenance boundary. The ISO 20022 tag mapping
is taken from BSE's own `SCRIP_STANDARDISED_FORMAT.docx` in the same archive.

## BSE outcome composition

Exclusions (14,808): non-equity security types (DB 8,826; GS 2,731; MF 2,266;
blank 8) and declared suspended status `S` (977).
Rejections (1,290): unclassified settlement groups `NS`, `NT`, `IF`, `F`, `W`
(751), duplicate scrip code or ISIN across BSE's legacy/new code ranges (516),
and invalid or missing ISIN (23, including literal `NA` and `0`).

Duplicate handling is deterministic and file-order first-wins: where one ISIN
appears under both a legacy `100xxx` scrip code and a newer `500xxx` code, the
first occurrence (the legacy `100xxx` code in this file) is retained and the
later one is rejected. The accepted set therefore carries the legacy code for
those securities; no identity is inferred or merged.

The tick size is read from field 58 `TickSz`, the authoritative tick-size field
in BSE's standardised contract. Field 9 `BidIntrvl` (the semantic successor of
v1's `NEW TICK SIZE`) is a fallback only; the two columns agree on every EQ row
in this fixture, so the choice does not affect these counts.

These are explicit, versioned rules; no assertion was relaxed to obtain the
pass. Unknown groups and undeclared source versions remain fail-closed.

## Limitations

Compatibility is established only for `nse-cash-equity-v1` and
`bse-security-master-v2` over the categories present in these two dated files.
It does not cover every exchange product, other dates, corporate-action
feeds, derivatives, or live market-data rights. `make check` was green on the
code commit; hosted CI results for the pull-request head are recorded in that
pull request's checks, not in this dated record.
