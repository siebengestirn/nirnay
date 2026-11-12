# Research fixtures

All files in this directory are **synthetic examples**. They demonstrate canonical event shape, reference-data lineage, and repeatable CLI wiring. They are not NSE observations, official instrument masters, exchange corporate-action notices, licensed data, or strategy-performance evidence.

- `canonical-events.example.jsonl`: canonical `MarketEvent` rows with synthetic capture-frame hashes.
- `reference-data.example.json`: bitemporal provider listing, universe membership, and a 2-for-1 split example. The SHA-256 values are fixture provenance IDs, not hashes of external official source documents.
- `alpha-bars.synthetic.csv`: deterministic close series for testing the bounded alpha-search CLI.

A real dataset must replace every fixture source with licensed, independently acquired source artifacts and accurate source-retrieval/knowledge timestamps. Do not label output built from these files as market data.
