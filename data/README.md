# Local data directory (not committed)

Suggested layout:

```text
data/raw/<provider>/<YYYY-MM-DD>/       append-only raw capture journal
 data/reference/                          point-in-time instrument/calendar/action snapshots
 data/datasets/<dataset_id>/              immutable Parquet files + manifest.json
 data/journals/                           local paper/account journals
 data/artifacts/                          experiment reports, plots and model artefacts
```

The actual entitlement, licence and retention rules for each feed determine
whether a dataset may be stored, analysed, backed up or redistributed. A broker
connection does not establish data redistribution rights. Never place tokens,
passwords, client secrets or signed URLs in this tree if it is backed up or
shared. Raw market data is excluded from Git by default.
