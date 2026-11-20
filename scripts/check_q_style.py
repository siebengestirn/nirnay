#!/usr/bin/env python3
"""Added-lines style gate for q / kdb+ (KDB-X) scripts.

Policy: engineering/q-kdb-style.md. Only mechanically unambiguous rules are
checked: whitespace hygiene, the silent-loader-stop bare `/` line, the
`count x < n` precedence bug, and dotted column access inside a lambda
(`t.col` is a global name in q; it must be `t[`col]`). Semantic rules such as
deterministic output are enforced by the q unit harness and Rust goldens.

Only lines added relative to a base ref are checked, so existing code is not
churned. No third-party dependencies.

Run from the repository root. Set NIRNAY_STYLE_BASE to compare against a
different ref (default: main). Use --self-test for adversarial unit tests.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import unittest

FILE_HEADER = re.compile(r"^\+\+\+ b/")
BARE_SLASH = re.compile(r"^/\s*$")
COUNT_COMPARISON = re.compile(r"\bcount\s+\w+\s*(?:<|>|<=|>=|=|~)")
# dotted names inside lambdas: t.col where both sides are plain identifiers
DOT_COLUMN = re.compile(r"(?<![\w.])\b([a-z][a-z0-9]{0,3})\.([a-z][a-z0-9_]*)\b")
MAX_COLUMNS = 100
TAB = re.compile(r"\t")
TRAILING_WS = re.compile(r"[ \t]+$")
# dotted globals that are namespaces, not column access
NAMESPACES = frozenset({".nirnay", ".z", ".Q", ".q", ".h", ".j", ".m", ".KU"})


def parse_added_lines(diff_text: str) -> dict[str, list[tuple[int, str]]]:
    """Parse a ``git diff --unified=0`` payload into added lines per file."""
    by_file: dict[str, list[tuple[int, str]]] = {}
    current = ""
    number = 0
    for line in diff_text.splitlines():
        if line.startswith("+++ "):
            current = line[6:] if FILE_HEADER.match(line) else ""
            number = 0
            continue
        if line.startswith("@@"):
            match = re.search(r"\+(\d+)", line)
            number = int(match.group(1)) if match else 0
            continue
        if line.startswith("+") and number:
            by_file.setdefault(current, []).append((number, line[1:]))
            number += 1
    return by_file


def collect_added_lines(base: str) -> dict[str, list[tuple[int, str]]]:
    result = subprocess.run(
        ["git", "diff", base, "--unified=0", "--", "*.q"],
        check=True,
        capture_output=True,
        text=True,
    )
    return parse_added_lines(result.stdout)


def check_q_line(line: str, *, in_lambda: bool = False) -> list[str]:
    """Return style findings for one q line.

    ``in_lambda`` reports whether the line is inside a `{...}` lambda, which
    is where dotted column access resolves to a global name instead of a
    column.
    """
    errors: list[str] = []
    stripped = line.strip()
    if TAB.search(line):
        errors.append("tab indentation")
    if TRAILING_WS.search(line):
        errors.append("trailing whitespace")
    if len(line) > MAX_COLUMNS:
        errors.append(f"line exceeds {MAX_COLUMNS} columns")
    if BARE_SLASH.match(stripped):
        errors.append("bare / line (silent loader stop)")
    if COUNT_COMPARISON.search(line):
        errors.append("unparenthesised comparison with count")
    if in_lambda and not stripped.startswith("/"):
        for match in DOT_COLUMN.finditer(stripped):
            dotted = match.group(0)
            if any(dotted.startswith(ns) for ns in NAMESPACES):
                continue
            # numeric literals such as 0.05 have a digit on the left
            if match.group(1)[0].isdigit():
                continue
            errors.append(f"dotted name {dotted!r} in lambda (use t[`col])")
    return errors


def check_lines(by_file: dict[str, list[tuple[int, str]]]) -> list[str]:
    """Check all added lines, tracking lambda depth per file."""
    errors: list[str] = []
    for path, lines in by_file.items():
        depth = 0
        for number, content in lines:
            code = content.split("/", 1)[0] if content.strip().startswith("/") else content
            # a single-line lambda f:{[t] t.col} opens and closes on this line
            in_lambda = depth > 0 or "{" in code
            for message in check_q_line(content, in_lambda=in_lambda):
                errors.append(f"{path}:{number}: {message}")
            depth += code.count("{") - code.count("}")
    return errors


class QStyleTests(unittest.TestCase):
    def test_bare_slash(self) -> None:
        self.assertTrue(any("bare /" in e for e in check_q_line("/")))

    def test_count_comparison(self) -> None:
        self.assertTrue(any("count" in e for e in check_q_line("if[count args < 2;x]")))

    def test_parenthesised_count_ok(self) -> None:
        self.assertEqual(check_q_line("if[(count args) < 2;x]"), [])

    def test_tab(self) -> None:
        self.assertTrue(any("tab" in e for e in check_q_line("\ta:1")))

    def test_trailing_whitespace(self) -> None:
        self.assertTrue(any("trailing" in e for e in check_q_line("a:1   ")))

    def test_long_line(self) -> None:
        self.assertTrue(any("columns" in e for e in check_q_line("a:" + "x" * 120)))

    def test_dotted_column_in_lambda(self) -> None:
        findings = check_q_line("last t.recv_ns", in_lambda=True)
        self.assertTrue(any("dotted name" in e for e in findings))

    def test_dotted_column_index_form_ok(self) -> None:
        self.assertEqual(check_q_line("last t[`recv_ns]", in_lambda=True), [])

    def test_namespace_dotted_ok(self) -> None:
        self.assertEqual(check_q_line(".nirnay.mbp.run[x]", in_lambda=True), [])

    def test_number_dotted_ok(self) -> None:
        self.assertEqual(check_q_line("0.05=x", in_lambda=True), [])

    def test_comment_exempt(self) -> None:
        self.assertEqual(check_q_line("/ t.recv_ns comment", in_lambda=True), [])

    def test_clean_line(self) -> None:
        self.assertEqual(check_q_line("a:1"), [])

    def test_diff_parser(self) -> None:
        diff = "+++ b/x.q\n@@ -0,0 +1,2 @@\n+a:1\n+b:2\n"
        self.assertEqual(parse_added_lines(diff), {"x.q": [(1, "a:1"), (2, "b:2")]})

    def test_check_lines_reports_path(self) -> None:
        errors = check_lines({"x.q": [(1, "/")]})
        self.assertTrue(any("x.q:1" in e for e in errors))

    def test_lambda_depth_tracking(self) -> None:
        lines = [(1, "f:{[t]"), (2, "  last t.recv_ns"), (3, "  };"), (4, "a:1")]
        errors = check_lines({"x.q": lines})
        self.assertTrue(any("x.q:2" in e for e in errors), "dotted column inside lambda")
        self.assertFalse(any("x.q:4" in e for e in errors), "top-level assignment clean")

    def test_single_line_lambda_caught(self) -> None:
        errors = check_lines({"x.q": [(1, "f:{[t] last t.recv_ns};")]})
        self.assertTrue(any("x.q:1" in e for e in errors), "single-line lambda dotted access")


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(QStyleTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    base = os.environ.get("NIRNAY_STYLE_BASE", "main")
    errors = check_lines(collect_added_lines(base))
    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        print(f"{len(errors)} q style violation(s) on changed lines")
        return 1
    print(f"PASS: q style guidance satisfied on changed lines vs {base}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
