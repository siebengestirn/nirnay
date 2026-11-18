# Real-source conformance record — 2026-11-18

**Prepared:** 2026-11-18 (operator-supplied dated exchange files).
**Revision:** `2ebacd7c6551621680636b643cd0bd60492caaff` (code commit; this
record is added in a separate evidence commit).
**Environment:** macOS 27.0.1 arm64; rustc/cargo 1.99.1.
**Scope:** compatibility of the two declared source versions and categories
listed below only. This is local engineering evidence, not a production,
market-data entitlement, exchange approval or profitability certification.

## Gate executed from the committed tree

```bash
NIRNAY_SOURCE_FIXTURES_DIR=/Users/tourist/Documents/nirnay-source-fixtures/2026-11-18 \
  cargo test -p india-reference --test source_conformance --offline -- --ignored --nocapture
```

Exit status: **0**. Result: `dated_exchange_sources_conform ... ok`
(`1 passed; 0 failed; 0 ignored; 1 filtered out`).
Output SHA-256:
`94fc53c571451432f6bba002ec1b8ce127621bb3308e47c75d1b3cd8b7365c52`.

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

These are explicit, versioned rules; no assertion was relaxed to obtain the
pass. Unknown groups and undeclared source versions remain fail-closed.

## Limitations

Compatibility is established only for `nse-cash-equity-v1` and
`bse-security-master-v2` over the categories present in these two dated files.
It does not cover every exchange product, other dates, corporate-action
feeds, derivatives, or live market-data rights. `make check` was green on the
code commit; hosted CI on the evidence commit is recorded separately.
