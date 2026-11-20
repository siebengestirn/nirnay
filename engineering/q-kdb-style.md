# Nirnay q / kdb+ (KDB-X) style guide

Canonical style rules for every `.q` file in the repository. This is a
companion to `STYLES.md` (which lists the upstream guides) and to
`engineering/style-policy.md` (which resolves cross-language conflicts).
The q rules here are adapted from the authoritative public sources below;
where they conflict, the Nirnay rules in this document win.

## Authoritative sources

- KX documentation links **"Remarks on Style"** from its Advanced q index:
  <https://github.com/qbists/style> (Stevan Apter, adapted for q).
- KX unit-testing Knowledge Base page documents the `k4unit` CSV-description
  model that the Nirnay q harness mirrors: <https://code.kx.com/q/kb/unit-tests/>.
- KX language reference for the operators whose precedence caused earlier
  bugs: [`deltas`](https://code.kx.com/q/ref/deltas/),
  [`string`](https://code.kx.com/q/ref/string/),
  [`0:`](https://code.kx.com/q/ref/file-text/),
  [`vs`](https://code.kx.com/q/ref/vs/).

## Nirnay rules (binding)

### N1 — One statement per line; one thought per statement

Follows *line-length.md*. Keep physical lines short (target ≤ 60 chars).
Comments are `/` lines above the statement they explain; a bare line
containing only `/` is forbidden because the q loader silently stops on it.

### N2 — No redundant trailing semicolons in scripts

Follows *semicolons.md*. Semicolons separate expressions inside a lambda,
but a top-level script line must not end in `;`.

### N3 — Whitespace only to reveal structure

Follows *whitespace.md*. No cosmetic spaces around operators; a single space
after a semicolon separator is allowed. Do not put spaces inside `f[x;y]`.

### N4 — Namespace globals, short local names

Global directories begin with a dot and a lowercase letter (`.nirnay.mbp`).
Function-valued globals begin with lowercase; locals are one or two letters.
Avoid `_` in names (it is the drop operator); use mixed case instead.
`x`, `y`, `z` are reserved for first/second/third lambda arguments only.

### N5 — Column access inside functions uses `t[\`col]`

In q, `t.col` is parsed as a dotted *global name* when `t` is a function
parameter, not as column access. Inside any lambda, always write `t[\`col]`.
This is enforced by the q unit harness and was the cause of a prior bug.

### N6 — Parenthesise comparisons against `count`

q parses `count args < 2` as `count (args < 2)`. Always write
`(count args) < 2`. This is enforced by `check_q_style.py`.

### N7 — `\\l` needs a literal; use `value "\\\\l ",string path`

The load command does not accept a variable path. Derive paths only from
`.z.f` (this script's own location) or validated arguments, then load via
`value "\\l ",string path`. Never load a path built from untrusted input.

### N8 — No network, broker, or write primitives in analytics

Analytics scripts contain no `hopen`, `set`, IPC handles or process-global
mutation. They read a CSV path, compute, and emit lines to stdout.

### N9 — Unit tests use the checked-in q harness

`analytics/kdb/mbp-test.q` runs the CSV descriptions in
`analytics/kdb/tests/*.csv`. Each case is `name,expression,expect` with
`expect` in {`true`,`false`}. `make kdb-q-test` runs it and fails on any
case. The harness is mirrored by Rust golden tests so CI works without q.

### N10 — Deterministic output is a contract

The emitted `metric,value` document is byte-pinned by
`apps/kdb-lab/tests/golden/mbp-metrics-v1.csv`. Changing the metric set,
order, or formatting requires regenerating that golden and updating the
Rust reference, the JSON schema, and the capability records in the same PR.

## Enforcement

- `scripts/check_q_style.py` checks added `.q` lines in `make check` and CI.
- `make kdb-q-test` runs real in-process q unit tests where q is installed.
- Rust golden tests and schema validation run everywhere, q or no q.
