# Ingress performance baseline — 2026-11-19

**Classification:** measured baseline on generated load, not a throughput
certificate or production claim. No operator-provided real provider capture
session was available, so all frames are synthetic; the sync_data failure model
of the real `JournalWriter` path is exercised on every append.

## Environment

- macOS 27.0.1, Darwin 27.0.0, arm64 (Apple M1-class host, 8 logical CPUs).
- Filesystem: APFS on `/System/Volumes/Data` (228 GiB, 62% used at capture).
- rustc/cargo 1.99.1; debug test profile; local temporary journal directory.
- Failure model: each append calls `sync_data()`; a failed write or sync poisons
  the writer. Replay streams with `JournalReader`, one frame held at a time.

## Measured results

| Case | Result |
|---|---|
| Append, 2,000 frames | 252.9 frames/s; p50 3,996.5 µs; p99 6,063.9 µs; p99.9 11,624.6 µs; queue depth 1; dropped 0; malformed 0 |
| Replay, 100,000 generated frames | 74,041.3 frames/s; peak frames held 1; delivery window 4,096; continuity streams 1,024 |
| Fresh-process recovery | resume of 100 committed frames in 0.008 s; child observed 100 frames |

`/usr/bin/time -l` around the replay test reported 3.88 s real, 3.73 s user,
0.07 s sys, 67,616,768 bytes maximum RSS and 43,172,488 bytes peak memory
footprint. That process includes generation of the 100,000-line journal, so
user time and RSS are upper bounds for replay alone.

## Commands and raw transcripts

```bash
cargo test -p market-archive --test performance --offline append_latency_and_throughput -- --ignored --nocapture
cargo test -p market-archive --test performance --offline replay_throughput_and_bounded_frames -- --ignored --nocapture
cargo test -p market-archive --test performance --offline recovery_after_process_termination -- --ignored --nocapture
/usr/bin/time -l cargo test -p market-archive --test performance --offline replay_throughput_and_bounded_frames -- --ignored --nocapture
```

| Transcript | SHA-256 |
|---|---|
| `ingress-append_latency_and_throughput.log` | `1613bdb38ff89db64f460693d2fdbd10ffd777fe9cd0dc342f3c3664fa3aa` |
| `ingress-replay_throughput_and_bounded_frames.log` | `5b737b5d562388d41db742f7f8cf0d61099a41a4dedf0dd3620bff56b3a475be` |
| `ingress-recovery_after_process_termination.log` | `ec849b802cac5e2c9fcacd342ecf8b42c44a259c1dcf3baba7eee6786895f623` |
| `ingress-replay-time.log` | `836174ef7b1962054528113ff48a31ee86b057499d7afd0d32acf7a8d23dd48a` |

## Interpretation

The synchronous append is dominated by per-record fsync; the ~4 ms p50 is an
explicit durability cost, not a market-data hot-path latency. Replay is bounded
by record size plus the fixed delivery/continuity budgets, not journal length.
No group commit, lock-free queue, Kafka, Redis or distributed store was added.
A production throughput certificate needs real captured sessions, a stated
filesystem and failure model, and a release-profile measurement.
