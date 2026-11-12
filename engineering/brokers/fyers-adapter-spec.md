# FYERS adapter implementation specification

**State:** proposed; no FYERS code ships in the first commit.

First implement a read-only recorder, not an order adapter. Verify the active
application tier, feed entitlements, permitted instruments/segments, historical
API limits, socket limits, retention and data-use terms directly in the account
and current official documentation before coding against assumed quotas.

The recorder must:

- keep credentials in environment/secret injection; never log auth URLs/tokens;
- record raw payload bytes before decoding, with source/receive timestamps,
  monotonic receive time, provider instrument key and schema version;
- record subscription intent, entitlement reference, reconnects, heartbeats,
  socket disconnects, decode failures and sequence/continuity observations;
- bound queues and explicitly record dropped/overflowed frames;
- distinguish exchange timestamp, provider timestamp and local receive time;
- identify stale instruments and invalid/out-of-order messages;
- write a durable journal and sync policy; crash/torn-tail tests are mandatory;
- produce a session quality report and immutable dataset source hashes;
- never turn an incomplete session into a clean dataset silently.

Before any live order functionality, separately verify broker retail-algo
requirements, static-IP registration, daily authentication, order-rate limits,
market-protection rules, tagging and the exact account/application category.
The adapter must not be treated as a legal-compliance abstraction.
