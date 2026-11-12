# ADR-0004: Live trading is disabled by construction

**Status:** Accepted.

The first repository commit has no live broker adapters. `configs/live.toml`
is a non-operational template with order submission disabled and zero limits.
Live routing requires broker entitlement verification, regulatory review,
independent risk authorization, durable OMS state, reconciliation, operational
monitoring, recovery tests and explicit human release approval.
