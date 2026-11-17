#!/usr/bin/env python3
"""Check Rust style guidance on changed lines.

Binding sources (see AGENTS.md):
  - Linux kernel Rust coding guidelines: https://docs.kernel.org/rust/coding-guidelines.html
  - Google Rust style guide: https://google.github.io/styleguide/rustguide.html

The check is deliberately limited to added lines relative to a base ref so
existing code is not churned. Run from the repository root. Set
NIRNAY_STYLE_BASE to compare against a different ref (default: main).
"""

from __future__ import annotations

import os
import re
import subprocess
import sys


def added_lines(base: str) -> dict[str, list[tuple[int, str]]]:
    result = subprocess.run(
        ["git", "diff", base, "--unified=0", "--", "*.rs"],
        check=True,
        capture_output=True,
        text=True,
    )
    by_file: dict[str, list[tuple[int, str]]] = {}
    current = ""
    for line in result.stdout.splitlines():
        if line.startswith("+++ b/"):
            current = line[6:]
            continue
        if line.startswith("@@"):
            match = re.search(r"\+(\d+)", line)
            number = int(match.group(1)) if match else 0
            continue
        if line.startswith("+") and not line.startswith("+++"):
            if current:
                by_file.setdefault(current, []).append((number, line[1:]))
            if number:
                number += 1
    return by_file


def main() -> int:
    base = os.environ.get("NIRNAY_STYLE_BASE", "main")
    errors: list[str] = []
    changed = added_lines(base)

    for path, lines in changed.items():
        text = "\n".join(content for _, content in lines)
        for number, content in lines:
            stripped = content.strip()

            # Google: no glob imports except the explicitly allowed test forms.
            if stripped.startswith("use ") and stripped.endswith("::*;"):
                if stripped not in {"use super::*;", "use crate::common::*;"}:
                    errors.append(f"{path}:{number}: glob import ({stripped!r})")

            # Kernel: multi-item imports are vertical, one item per line, with
            # a trailing empty comment to preserve the layout under rustfmt.
            if re.match(r"^use [A-Za-z0-9_:]+::\{[^}]*,[^}]*\};", stripped):
                errors.append(f"{path}:{number}: multi-item import must be vertical ({stripped!r})")

            # Kernel: prefer #[expect] over #[allow]; an allow needs a reason.
            if stripped.startswith("#[allow(") and "//" not in stripped:
                errors.append(f"{path}:{number}: #[allow] without inline justification")

        # Google: assertion macros should carry a context string. The crude
        # scan accepts any string literal in the macro invocation.
        for match in re.finditer(r"assert(?:_eq|_ne|_matches!)?!\(", text):
            segment = text[match.start(): match.start() + 600]
            depth = 0
            end = len(segment)
            for index, char in enumerate(segment):
                if char == "(":
                    depth += 1
                elif char == ")":
                    depth -= 1
                    if depth == 0:
                        end = index
                        break
            invocation = segment[:end]
            if not re.search(r'"[^"]*"', invocation):
                errors.append(f"{path}: assertion without context string")

    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        print(f"{len(errors)} Rust style violation(s) on changed lines")
        return 1
    print(f"PASS: Rust style guidance satisfied on changed lines vs {base}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
