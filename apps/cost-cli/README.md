# cost-cli

A JSON-lines bridge to the existing Rust `cost-model`. It ensures research
software calls the authoritative effective-dated charge implementation rather
than maintaining a second fee formula.

```bash
cargo run -p cost-cli -- --schedule configs/your-authorised-rates.json < trades.jsonl
```

Input fields: `trade_date` (`YYYY-MM-DD`), `product` (`delivery` or `intraday`),
`side` (`Buy` or `Sell`, matching the domain enum), and `turnover_paise` (integer).
Output is integer paise for every fee component and the total.

No production tariff is bundled. The operator must maintain effective-dated
rates from verified official/broker sources and keep provenance in a separate
rate manifest. Missing schedule is an error, never zero fees. Test rates in the
unit tests are synthetic and must never be used for trading.
