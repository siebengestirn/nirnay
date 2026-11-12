# ADR-0001: Modular monolith first

**Status:** Accepted for the first development stage.

Use one Rust workspace with explicit dependency layers and separate Python
research tooling. Do not introduce microservices, Kafka, Redis, Kubernetes,
C++ acceleration or kdb+/q without a measured requirement that cannot be met
by the local modular monolith.

The applications are composition roots. Domain, storage, risk and simulation
remain testable libraries. A crate boundary must correspond to a stable contract,
independent ownership, meaningful dependency rule or separate verification need.
