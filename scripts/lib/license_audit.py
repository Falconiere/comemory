#!/usr/bin/env python3
"""Fail when a resolved crate's license is not on deny.toml's allow list.

Reads `cargo metadata --format-version 1` on stdin and the path to deny.toml
as argv[1]. A crate's `license` field is an SPDX expression; this treats the
`OR` / `AND` / parenthesised forms the Rust registry actually uses:

* `A OR B`  -> satisfied when ANY branch is allowed (a dual-licensed crate can
  be taken under whichever branch is permitted).
* `A AND B` -> satisfied only when EVERY branch is allowed.

A crate with a null `license` and a `license_file` is reported, not silently
accepted: an unreviewed bespoke license is exactly what the gate is for. The
workspace's own crates are skipped (they are not third-party dependencies).
"""

import json
import re
import sys
import tomllib


def allowed_licenses(deny_toml_path):
    """Read the `allow` list out of deny.toml's [licenses] table.

    Parsed with `tomllib` rather than a regex: a hand-rolled
    `allow\\s*=\\s*\\[(.*?)\\]` stops at the first `]`, so any nesting or a
    `]` inside a comment silently truncates the set — and a license that
    falls off the allow list makes the audit pass things it should reject.
    """
    with open(deny_toml_path, "rb") as handle:
        document = tomllib.load(handle)
    licenses = document.get("licenses")
    if not isinstance(licenses, dict):
        raise SystemExit("license-audit: deny.toml has no [licenses] table")
    allow = licenses.get("allow")
    if not isinstance(allow, list) or not allow:
        raise SystemExit("license-audit: deny.toml [licenses] has no allow list")
    return {entry for entry in allow if isinstance(entry, str)}


def split_top_level(expression, operator):
    """Split on `operator` only at paren depth 0, preserving sub-expressions.

    `operator` is "OR" or "AND". The legacy registry forms `MIT/Apache-2.0`
    and `Apache-2.0 / MIT` mean OR, so a slash is treated as an OR separator.
    """
    parts, buffer, depth, index = [], "", 0, 0
    while index < len(expression):
        char = expression[index]
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
        if depth == 0:
            if operator == "OR" and char == "/":
                parts.append(buffer)
                buffer = ""
                index += 1
                continue
            match = re.match(rf"(?:^|(?<=[\s)]))\s*{operator}\s+", expression[index:])
            if match and (index == 0 or expression[index - 1] in " \t)"):
                parts.append(buffer)
                buffer = ""
                index += match.end()
                continue
        buffer += char
        index += 1
    parts.append(buffer)
    return [part.strip() for part in parts if part.strip()]


def satisfied(expression, allow):
    """Whether an SPDX expression is satisfiable from `allow`."""
    expression = expression.strip()
    while expression.startswith("(") and expression.endswith(")"):
        inner = expression[1:-1]
        # Only strip when those parens actually wrap the whole expression,
        # so "(A OR B) AND (C)" is not mangled into "A OR B) AND (C".
        if split_top_level(inner, "OR") == [inner] or inner.count("(") == inner.count(")"):
            depth = 0
            wraps = True
            for position, char in enumerate(inner):
                depth += (char == "(") - (char == ")")
                if depth < 0:
                    wraps = False
                    break
            if not wraps:
                break
            expression = inner.strip()
        else:
            break
    # OR binds loosest, so split on it first; any satisfied branch is enough.
    branches = split_top_level(expression, "OR")
    if len(branches) > 1:
        return any(satisfied(branch, allow) for branch in branches)
    branches = split_top_level(expression, "AND")
    if len(branches) > 1:
        return all(satisfied(branch, allow) for branch in branches)
    return expression.rstrip("+") in allow or expression in allow


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: license_audit.py <path-to-deny.toml>")
    allow = allowed_licenses(sys.argv[1])
    metadata = json.load(sys.stdin)
    workspace = set(metadata.get("workspace_members", []))
    offenders = []
    for package in metadata["packages"]:
        if package["id"] in workspace:
            continue
        license_expression = package.get("license")
        label = f"{package['name']} {package['version']}"
        if not license_expression:
            offenders.append(f"{label}: no license field (license_file only)")
        elif not satisfied(license_expression, allow):
            offenders.append(f"{label}: {license_expression}")
    if offenders:
        print("license-audit: licenses outside deny.toml's allow list:", file=sys.stderr)
        for offender in sorted(offenders):
            print(f"  {offender}", file=sys.stderr)
        raise SystemExit(1)
    print(f"license-audit: {len(metadata['packages'])} crates, all allowed")


if __name__ == "__main__":
    main()
