#!/usr/bin/env python3
"""Added-lines style gate for every non-Rust language in the repository.

Rust has its own gate (`check_rust_style.py`) plus rustfmt and Clippy. This
script applies the same idea to Python, C/C++, Makefiles, JSON/JSONL and
Markdown: only lines added relative to a base ref are checked, so existing code
is not churned. It has no third-party dependencies and is binding in
`make check`.

Policy sources and the repository's conflict resolutions are recorded in
`engineering/style-policy.md`. The authoritative formatters remain
`clang-format` for C/C++ (pinned in CI) and `ruff` for Python (opt-in until the
existing corpus is reformatted in a dedicated PR).

Run from the repository root. Set `NIRNAY_STYLE_BASE` to compare against a
different ref (default: `main`). Use `--self-test` for adversarial unit tests.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import unittest
from pathlib import Path

MAX_PY_COLUMNS = 100
MAX_CPP_COLUMNS = 100
MAX_MD_COLUMNS = 100

CPP_EXTS = frozenset({".c", ".cc", ".cpp", ".cxx", ".h", ".hpp", ".hxx"})
HEADER_EXTS = frozenset({".h", ".hpp", ".hxx"})
JSON_EXTS = frozenset({".json"})
JSONL_EXTS = frozenset({".jsonl"})
MD_EXTS = frozenset({".md"})

FILE_HEADER = re.compile(r"^\+\+\+ b/")
HUNK_HEADER = re.compile(r"^@@.*\+(\d+)")
HEADING_NEEDS_SPACE = re.compile(r"^\s*#{1,6}[^#\s]")
BARE_EXCEPT = re.compile(r"^\s*except\s*:")
WILDCARD_IMPORT = re.compile(r"^\s*from\s+\S+\s+import\s+\*")
BACKSLASH_CONTINUATION = re.compile(r"\\\s*$")
MUTABLE_DEFAULT = re.compile(r"\bdef\s+\w+\s*\([^)]*=\s*(\[\]|\{\})")
NULL_LITERAL = re.compile(r"\bNULL\b")
USING_NAMESPACE_STD = re.compile(r"^\s*using\s+namespace\s+std\s*;")
TARGET_LINE = re.compile(r"^[^#\t][^:=]*:(?:=|\+|\?|;|\s|$)")


def extension_of(path: str) -> str:
    return Path(path).suffix.lower()


def is_makefile(path: str) -> bool:
    return Path(path).name == "Makefile" or Path(path).name.endswith(".mk")


def is_supported(path: str) -> bool:
    ext = extension_of(path)
    return (
        ext == ".py"
        or ext in CPP_EXTS
        or ext in JSON_EXTS
        or ext in JSONL_EXTS
        or ext in MD_EXTS
        or is_makefile(path)
    )


def parse_added_lines(diff_text: str) -> dict[str, list[tuple[int, str]]]:
    """Parse `git diff --unified=0` into added lines keyed by path."""
    by_file: dict[str, list[tuple[int, str]]] = {}
    current = ""
    number = 0
    for line in diff_text.splitlines():
        if line.startswith("+++ "):
            current = line[6:] if FILE_HEADER.match(line) else ""
            number = 0
            continue
        if line.startswith("@@"):
            match = HUNK_HEADER.match(line)
            number = int(match.group(1)) if match else 0
            continue
        if line.startswith("+") and number and current:
            by_file.setdefault(current, []).append((number, line[1:]))
            number += 1
    return by_file


def collect_added_lines(base: str) -> dict[str, list[tuple[int, str]]]:
    result = subprocess.run(
        ["git", "diff", base, "--unified=0"],
        check=True,
        capture_output=True,
        text=True,
    )
    added = parse_added_lines(result.stdout)
    return {path: lines for path, lines in added.items() if is_supported(path)}


def trailing_whitespace(line: str) -> bool:
    return line.rstrip() != line


def check_python_line(line: str) -> list[str]:
    errors: list[str] = []
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    if re.match(r"^\s*\t", line):
        errors.append("tab indentation; use four spaces")
    if len(line) > MAX_PY_COLUMNS and "http://" not in line and "https://" not in line:
        errors.append(f"line exceeds {MAX_PY_COLUMNS} columns")
    if ";" in line and re.search(r";\s*$", line) and not line.lstrip().startswith("#"):
        errors.append("do not terminate a statement with a semicolon")
    if BARE_EXCEPT.match(line):
        errors.append("bare `except:` is forbidden; catch a specific exception")
    if WILDCARD_IMPORT.match(line):
        errors.append("wildcard import is forbidden")
    if BACKSLASH_CONTINUATION.search(line):
        errors.append("explicit backslash continuation; use implicit joining")
    if MUTABLE_DEFAULT.search(line):
        errors.append("mutable default argument; use None")
    return errors


def check_cpp_line(line: str, *, header: bool, cpp: bool) -> list[str]:
    errors: list[str] = []
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    if "\t" in line:
        errors.append("tab character; .clang-format uses spaces")
    if len(line) > MAX_CPP_COLUMNS:
        errors.append(f"line exceeds {MAX_CPP_COLUMNS} columns")
    if cpp and NULL_LITERAL.search(line):
        errors.append("use nullptr instead of NULL")
    if header and USING_NAMESPACE_STD.match(line):
        errors.append("using-directive is forbidden in a header")
    return errors


def check_makefile_line(line: str) -> list[str]:
    errors: list[str] = []
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    if "\r" in line:
        errors.append("CRLF line ending; use LF")
    return errors


def makefile_recipe_violations(text: str) -> set[int]:
    """Line numbers where a recipe command is indented with spaces."""
    violations: set[int] = set()
    lines = text.splitlines()
    previous_was_target = False
    for number, line in enumerate(lines, start=1):
        if not line.strip() or line.lstrip().startswith("#"):
            previous_was_target = False
            continue
        if line.startswith("\t"):
            previous_was_target = False
            continue
        if previous_was_target and line.startswith(" "):
            violations.add(number)
        previous_was_target = bool(TARGET_LINE.match(line))
    return violations


def markdown_fence_lines(text: str) -> tuple[set[int], set[int], set[int]]:
    """Return (fence markers opening without a language, fenced-content lines)."""
    bare_openings: set[int] = set()
    content: set[int] = set()
    inside = False
    for number, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if stripped.startswith("```"):
            if not inside and len(stripped) == 3:
                bare_openings.add(number)
            inside = not inside
            continue
        if inside:
            content.add(number)
    return bare_openings, content


def check_markdown_line(
    line: str,
    *,
    in_fence: bool,
    bare_fence_opening: bool,
    is_fence_marker: bool,
) -> list[str]:
    errors: list[str] = []
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    if bare_fence_opening:
        errors.append("declare a language on fenced code blocks")
    if in_fence or is_fence_marker:
        return errors
    if HEADING_NEEDS_SPACE.match(line):
        errors.append("ATX heading needs a space after `#`")
    is_heading = bool(re.match(r"^#{1,6}\s", line))
    is_table = "|" in line
    has_link = "](" in line or "http://" in line or "https://" in line
    if len(line) > MAX_MD_COLUMNS and not (is_heading or is_table or has_link):
        errors.append(f"prose line exceeds {MAX_MD_COLUMNS} columns")
    return errors


def check_changed_file(path: str, added: list[tuple[int, str]]) -> dict[int, list[str]]:
    """Return violations by line number for one changed file."""
    errors: dict[int, list[str]] = {}
    add = lambda number, message: errors.setdefault(number, []).append(message)  # noqa: E731
    ext = extension_of(path)
    added_numbers = {number for number, _ in added}

    if ext == ".py":
        for number, line in added:
            for message in check_python_line(line):
                add(number, message)

    elif ext in CPP_EXTS:
        for number, line in added:
            for message in check_cpp_line(
                line, header=ext in HEADER_EXTS, cpp=ext not in {".c"}
            ):
                add(number, message)

    elif is_makefile(path):
        for number, line in added:
            for message in check_makefile_line(line):
                add(number, message)
        try:
            text = Path(path).read_text()
        except OSError:
            return errors
        for number in makefile_recipe_violations(text) & added_numbers:
            add(number, "recipe command must start with a tab")

    elif ext in JSON_EXTS:
        try:
            text = Path(path).read_text()
        except OSError:
            return errors
        try:
            json.loads(text)
        except json.JSONDecodeError as error:
            add(error.lineno, f"invalid JSON: {error.msg}")
        lines = text.splitlines()
        if text and not text.endswith("\n"):
            add(len(lines), "missing final newline")
        for number, line in added:
            if number > len(lines):
                continue
            current = lines[number - 1]
            if trailing_whitespace(current):
                add(number, "trailing whitespace")
            indent = len(current) - len(current.lstrip(" "))
            if "\t" in current[:indent]:
                add(number, "tab indentation; JSON uses two spaces")
            elif indent % 2:
                add(number, "indentation must be a multiple of two spaces")

    elif ext in JSONL_EXTS:
        try:
            text = Path(path).read_text()
        except OSError:
            return errors
        for number, line in added:
            if not line.strip():
                continue
            if trailing_whitespace(line):
                add(number, "trailing whitespace")
            try:
                json.loads(line)
            except json.JSONDecodeError as error:
                add(number, f"invalid JSON record: {error.msg}")
        if text and not text.endswith("\n"):
            add(len(text.splitlines()), "missing final newline")

    elif ext in MD_EXTS:
        try:
            text = Path(path).read_text()
        except OSError:
            return errors
        bare_openings, content = markdown_fence_lines(text)
        lines = text.splitlines()
        fully_added = len(added_numbers) >= len(lines)
        for number, line in added:
            if number > len(lines):
                continue
            for message in check_markdown_line(
                line,
                in_fence=number in content,
                bare_fence_opening=number in bare_openings,
                is_fence_marker=bool(line.strip().startswith("```")),
            ):
                add(number, message)
        if fully_added:
            headings = [
                number
                for number, line in enumerate(lines, start=1)
                if line.startswith("# ")
            ]
            if len(headings) > 1:
                for number in headings[1:]:
                    add(number, "document has more than one H1 heading")

    return errors


class StyleCheckerTests(unittest.TestCase):
    def test_parser_handles_multiple_files_and_hunks(self) -> None:
        diff = """+++ b/a.py
@@ -1,1 +1,2 @@
 context
+good
@@ -10,1 +11,1 @@
-old
+new
+++ /dev/null
@@ -1,1 +0,0 @@
-removed
+++ b/src/b.cpp
@@ -0,0 +1,1 @@
+int x = 1
"""
        self.assertEqual(
            parse_added_lines(diff),
            {"a.py": [(1, "good"), (11, "new")], "src/b.cpp": [(1, "int x = 1")]},
        )

    def test_python_rules(self) -> None:
        self.assertEqual(check_python_line("x = 1"), [])
        self.assertIn("trailing whitespace", check_python_line("x = 1 "))
        self.assertIn("tab indentation; use four spaces", check_python_line("\tx = 1"))
        self.assertIn("do not terminate a statement with a semicolon", check_python_line("x = 1;"))
        self.assertTrue(any("bare" in m for m in check_python_line("except:")))
        self.assertTrue(any("wildcard" in m for m in check_python_line("from x import *")))
        self.assertTrue(any("backslash" in m for m in check_python_line("x = 1 + \\")))
        self.assertTrue(
            any("mutable" in m for m in check_python_line("def f(x=" + "[]" + "):"))
        )

    def test_python_allows_urls_and_string_semicolons(self) -> None:
        long_url = "x = 'https://example.com/" + "a" * 120 + "'"
        self.assertEqual(check_python_line(long_url), [])
        self.assertEqual(check_python_line("text = 'a;b'"), [])

    def test_cpp_rules(self) -> None:
        self.assertEqual(check_cpp_line("int x;", header=False, cpp=True), [])
        self.assertTrue(
            any("nullptr" in m for m in check_cpp_line("x = NULL;", header=False, cpp=True))
        )
        self.assertTrue(
            any(
                "using-directive" in m
                for m in check_cpp_line("using namespace std;", header=True, cpp=True)
            )
        )
        self.assertEqual(check_cpp_line("using namespace std;", header=False, cpp=True), [])
        self.assertTrue(
            any("tab" in m for m in check_cpp_line("\tint x;", header=False, cpp=True))
        )

    def test_makefile_recipe_detection(self) -> None:
        text = "all: build\n\tmake build\nbuild:\n  echo wrong\n"
        self.assertEqual(makefile_recipe_violations(text), {4})

    def test_markdown_fences_and_headings(self) -> None:
        text = "# Title\n\n```\ncode\n```\n\n##Bad\n"
        bare_openings, content = markdown_fence_lines(text)
        self.assertEqual(bare_openings, {3})
        self.assertEqual(content, {4})
        self.assertTrue(
            any(
                "language" in m
                for m in check_markdown_line(
                    "```", in_fence=False, bare_fence_opening=True, is_fence_marker=True
                )
            )
        )
        self.assertTrue(
            any(
                "space" in m
                for m in check_markdown_line(
                    "##Bad", in_fence=False, bare_fence_opening=False, is_fence_marker=False
                )
            )
        )

    def test_json_and_jsonl(self) -> None:
        self.assertEqual(json.loads('{"a": 1}'), {"a": 1})
        with self.assertRaises(json.JSONDecodeError):
            json.loads("{ // comment\n}")

    def test_supported_extensions(self) -> None:
        self.assertTrue(is_supported("a.py"))
        self.assertTrue(is_supported("Makefile"))
        self.assertTrue(is_supported("x.hxx"))
        self.assertTrue(is_supported("a.jsonl"))
        self.assertFalse(is_supported("image.png"))


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(StyleCheckerTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1

    base = os.environ.get("NIRNAY_STYLE_BASE", "main")
    by_file = collect_added_lines(base)
    failures: list[str] = []
    for path, added in sorted(by_file.items()):
        for number, messages in sorted(check_changed_file(path, added).items()):
            for message in messages:
                failures.append(f"{path}:{number}: {message}")
    if failures:
        for failure in failures:
            print(f"FAIL: {failure}")
        print(f"{len(failures)} source style violation(s) on added lines")
        return 1
    print(f"PASS: source style guidance satisfied on added lines vs {base}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
