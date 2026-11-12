# Security and trading safety

This repository is research/paper infrastructure. It does not ship live broker
order routing. Report vulnerabilities privately to the repository owner before
public disclosure.

Never store API tokens, TOTP seeds, private keys, account numbers or signed URLs
in source control, logs, fixtures or experiment artifacts. Use a local secret
manager or environment injection and redact all request/response diagnostics.

Any future live integration must fail closed on stale data, unknown broker order
state, missing risk authorization, reconciliation mismatch, rate-limit breach,
configuration drift or unhealthy persistence. A kill switch must be independent
of strategy code. Live deployment requires a separate reviewed release process.
