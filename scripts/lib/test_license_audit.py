#!/usr/bin/env python3
"""Real-data tests for the SPDX expression handling in license_audit.py.

Every expression below is a literal `license` string taken from this repo's
own resolved dependency tree (`cargo metadata --all-features`), and the allow
set is parsed from the real deny.toml — no synthesized fixtures, so a registry
form the tree actually contains cannot pass here and fail in the gate.

Run: python3 scripts/lib/test_license_audit.py
"""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from license_audit import allowed_licenses, satisfied, split_top_level

DENY_TOML = pathlib.Path(__file__).resolve().parents[2] / "deny.toml"

# Every distinct `license` value in this repo's resolved tree, verbatim from
# `cargo metadata --all-features`. This is the whole real corpus, so a form the
# tree contains cannot pass the gate while failing here, or vice versa.
REAL_ALLOWED = [
    "(MIT OR Apache-2.0) AND Unicode-3.0",
    "Apache-2.0",
    "Apache-2.0 / MIT",
    "Apache-2.0 AND ISC",
    "Apache-2.0 OR BSL-1.0",
    "Apache-2.0 OR ISC OR MIT",
    "Apache-2.0 OR MIT",
    "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
    "BSD-2-Clause OR Apache-2.0 OR MIT",
    "BSD-3-Clause",
    "CC0-1.0 OR MIT-0 OR Apache-2.0",
    "CDLA-Permissive-2.0",
    "ISC",
    "ISC AND (Apache-2.0 OR ISC)",
    "ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause "
    "AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0)",
    "MIT",
    "MIT AND BSD-3-Clause",
    "MIT OR Apache-2.0",
    "MIT OR Apache-2.0 OR LGPL-2.1-or-later",
    "MIT OR Apache-2.0 OR Zlib",
    "MIT/Apache-2.0",
    "Unicode-3.0",
    "Unlicense OR MIT",
    "Unlicense/MIT",
    "Zlib",
    "Zlib OR Apache-2.0 OR MIT",
]

# Expressions that must be rejected: none is satisfiable from the allow list.
REAL_REJECTED = [
    "GPL-3.0",
    "AGPL-3.0-only",
    "GPL-2.0 AND MIT",
    "Unlicense",
    "(GPL-3.0 OR LGPL-3.0)",
]


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def test_split_top_level_respects_parens():
    check(
        split_top_level("ISC AND (Apache-2.0 OR ISC)", "AND")
        == ["ISC", "(Apache-2.0 OR ISC)"],
        "AND split must not descend into parentheses",
    )
    check(
        split_top_level("(MIT OR Apache-2.0) AND Unicode-3.0", "OR")
        == ["(MIT OR Apache-2.0) AND Unicode-3.0"],
        "a parenthesised OR is not a top-level OR",
    )
    check(
        split_top_level("MIT/Apache-2.0", "OR") == ["MIT", "Apache-2.0"],
        "the legacy slash form is an OR separator",
    )
    check(
        split_top_level("Apache-2.0 / MIT", "OR") == ["Apache-2.0", "MIT"],
        "a spaced slash is an OR separator too",
    )


def test_real_tree_expressions_are_allowed():
    allow = allowed_licenses(DENY_TOML)
    for expression in REAL_ALLOWED:
        check(
            satisfied(expression, allow),
            f"expected allowed, got rejected: {expression!r}",
        )


def test_disallowed_expressions_are_rejected():
    allow = allowed_licenses(DENY_TOML)
    for expression in REAL_REJECTED:
        check(
            not satisfied(expression, allow),
            f"expected rejected, got allowed: {expression!r}",
        )


def test_and_requires_every_branch():
    # ISC is allowed and GPL-3.0 is not, so the conjunction must fail even
    # though one side passes -- the bug that would let a copyleft crate in.
    allow = allowed_licenses(DENY_TOML)
    check(
        not satisfied("ISC AND GPL-3.0", allow),
        "AND must require every branch to be allowed",
    )


def main():
    tests = [value for name, value in sorted(globals().items()) if name.startswith("test_")]
    for test in tests:
        test()
        print(f"ok  {test.__name__}")
    print(f"{len(tests)} passed")


if __name__ == "__main__":
    main()
