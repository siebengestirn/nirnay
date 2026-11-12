# Strategy validation policy

1. Register the hypothesis, economic rationale, universe, holding period, features,
   parameters, benchmarks and falsification rule before the search begins.
2. Freeze dataset identity, schema, point-in-time reference data, source hashes,
   code revision and full configuration for every experiment.
3. Preserve rejected and failed trials. Record the full number of ideas and
   parameter combinations tested; do not report only the winner.
4. Prevent look-ahead, survivorship and corporate-action errors. A current
   constituent list is not a historical universe.
5. Use effective-dated brokerage, taxes, exchange fees, stamp duty and GST.
   Slippage and execution assumptions are separate from regulatory fees.
6. Do not assume a bar touched a limit price means the order filled. Simulate
   spread, queue uncertainty, partial fills, rejects, latency and liquidity.
7. Use chronological walk-forward validation and a final untouched holdout.
   Correct inference for multiple trials and regime selection.
8. Compare against simple baselines and report gross and net results separately.
9. Paper-trade the frozen candidate against live data. Re-estimate execution
   differences before any human-approved live release.
10. A backtest, AI assessment, paper return or software test is not evidence of
    future profitability.
