# C++ numeric/order-book oracle (verification only)

This target promotes the predecessor's standalone exact-numeric and
aggregated-book implementation out of the predecessor tree (formerly
`legacy/`, now archived under `.local/extras/`) so it can remain a separately
built verification reference. It is **not linked into the production Rust workspace**, is not a
broker adapter, and is not a source of truth for trading/accounting semantics.

The source was promoted from the predecessor tree:

- `numeric.h` and `numeric.cpp`
- `market_state.h` and `market_state.cpp`
- `tests/numeric_orderbook_test.cpp`

Those canonical copies now live only in this directory. The duplicate copies
were removed from `.local/extras/cpp`; that archived directory retains only
the SBE and conformance probes and links against these sources.

The test covers fixed-point parse/format/rescale, integer overflow boundaries, rational exactness,
order-book level updates, tick grids, quantity steps and lot sizes. The implementation uses
compiler-supported `__int128` for checked intermediate arithmetic; this verification target requires
GCC or Clang and makes no claim of ISO C++ portability.

Build and run independently:

```bash
cmake -S verification/cpp-oracle -B /tmp/nirnay-cpp-oracle -DCMAKE_BUILD_TYPE=Release
cmake --build /tmp/nirnay-cpp-oracle --parallel
ctest --test-dir /tmp/nirnay-cpp-oracle --output-on-failure
```

A true differential oracle requires the Rust and C++ drivers to consume the same versioned,
language-neutral corpus and compare canonical outputs. This promotion provides the independent C++
executable and boundary tests, but it does **not** yet assert byte-for-byte equivalence with every
active Rust implementation. Do not use a C++ result to overrule an accepted Rust/domain contract
without a reviewed ADR.
