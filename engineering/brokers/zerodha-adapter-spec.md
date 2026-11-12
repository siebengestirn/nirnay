# Zerodha adapter implementation specification

**State:** proposed; no Zerodha code ships in the first commit.

Treat Zerodha as a separate execution route. Do not assume the free Personal API
includes market data. Confirm the active account entitlement and pricing for
any live/historical feed separately. The adapter must implement authentication,
rate limiting, order submission/cancel/status, order updates, positions, funds,
reconciliation and error classification behind a broker-neutral port.

A timeout following order submission produces `Unknown` until the order book or
broker order history has been reconciled. Never retry a potentially accepted
order blindly. Use broker identifiers plus local idempotency keys. Reconcile
cash, positions, open orders and fills before enabling a strategy after restart.

No credentials, API tokens, TOTP secrets or account identifiers belong in Git,
fixtures, logs or test output. Live execution remains blocked until current
broker/exchange rules are checked for the exact account and use case.
