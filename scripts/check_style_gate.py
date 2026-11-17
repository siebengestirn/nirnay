#!/usr/bin/env python3
"""Binding integrity check for the style gate itself.

A formatter gate is only useful if nobody can silently remove or weaken it.
This checker asserts that the Makefile, CI workflow and both style checkers
still contain the required wiring, language coverage and test inventory. It is
itself invoked from `make check`, from the CI `source-style` job and from the
independent `python-research` CI job, so disabling one path is not enough.

Adding a new language or rule requires updating the expectations here in the
same PR; removing a rule or a test below the declared minimum fails the gate.
"""

from __future__ import annotations

import re
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

REQUIRED_TEXT: dict[str, tuple[str, ...]] = {
    "Makefile": (
        "style: rust-style source-style style-gate",
        "$(MAKE) style",
        "python3 scripts/check_rust_style.py --self-test",
        "python3 scripts/check_source_style.py --self-test",
        "python3 scripts/check_style_gate.py",
    ),
    ".github/workflows/ci.yml": (
        "source-style:",
        "fetch-depth: 0",
        "NIRNAY_STYLE_BASE: origin/main",
        "python3 scripts/check_rust_style.py",
        "python3 scripts/check_source_style.py",
        "python3 scripts/check_style_gate.py",
    ),
    "scripts/check_rust_style.py": (
        "ASSERT_MACRO",
        "--self-test",
        "def check_lines(",
        "def main()",
    ),
    "scripts/check_source_style.py": (
        "def check_python_line(",
        "def check_cpp_line(",
        "def check_shell_line(",
        "def check_makefile_line(",
        "def check_json_line(",
        "def check_markdown_line(",
        "SHELL_EXTS",
        "--self-test",
        "def main()",
    ),
}

EXPECTED_LANGUAGE_FUNCTIONS: tuple[str, ...] = (
    "def check_python_line(",
    "def check_cpp_line(",
    "def check_shell_line(",
    "def check_makefile_line(",
    "def check_json_line(",
    "def check_markdown_line(",
)

MIN_TESTS: dict[str, int] = {
    "scripts/check_rust_style.py": 13,
    "scripts/check_source_style.py": 23,
    "scripts/check_style_gate.py": 4,
}


def validate(root: Path = ROOT) -> list[str]:
    """Return one message per missing or weakened gate requirement."""
    errors: list[str] = []
    for relative, needles in REQUIRED_TEXT.items():
        path = root / relative
        if not path.is_file():
            errors.append(f"{relative}: required gate file is missing")
            continue
        text = path.read_text()
        for needle in needles:
            if needle not in text:
                errors.append(f"{relative}: missing required gate text {needle!r}")
    for relative, minimum in MIN_TESTS.items():
        path = root / relative
        if not path.is_file():
            continue
        tests = len(re.findall(r"^\s+def test_", path.read_text(), re.M))
        if tests < minimum:
            errors.append(f"{relative}: {tests} self-tests, expected at least {minimum}")
    return errors


class GateIntegrityTests(unittest.TestCase):
    def fake_root(self, directory: str) -> Path:
        root = Path(directory)
        for relative in REQUIRED_TEXT:
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                "\n".join(REQUIRED_TEXT[relative])
                + "\n"
                + "    def test_example() -> None:\n        pass\n" * MIN_TESTS.get(relative, 1)
            )
        return root

    def test_language_coverage_cannot_be_removed(self) -> None:
        required = REQUIRED_TEXT["scripts/check_source_style.py"]
        for needle in EXPECTED_LANGUAGE_FUNCTIONS:
            self.assertIn(needle, required)
        self.assertEqual(len(EXPECTED_LANGUAGE_FUNCTIONS), 6)

    def test_complete_gate_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(validate(self.fake_root(directory)), [])

    def test_removed_makefile_wiring_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.fake_root(directory)
            path = root / "Makefile"
            path.write_text(path.read_text().replace("$(MAKE) style", ""))
            self.assertTrue(any("Makefile" in e for e in validate(root)))

    def test_removed_language_rule_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.fake_root(directory)
            path = root / "scripts/check_source_style.py"
            path.write_text(path.read_text().replace("def check_shell_line(", ""))
            self.assertTrue(any("check_shell_line" in e for e in validate(root)))

    def test_removed_self_test_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.fake_root(directory)
            path = root / "scripts/check_source_style.py"
            lines = path.read_text().splitlines()
            lines = [line for line in lines if "def test_" not in line]
            path.write_text("\n".join(lines))
            self.assertTrue(any("self-tests" in e for e in validate(root)))


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(GateIntegrityTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    errors = validate()
    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        print(f"{len(errors)} style-gate integrity violation(s)")
        return 1
    print("PASS: style gate wiring and rule inventory are intact")
    return 0


if __name__ == "__main__":
    sys.exit(main())
