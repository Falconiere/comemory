"""A block-YAML emitter with every scalar explicitly typed.

Written here rather than taken from PyYAML because every other component of this
harness is standard library only, and one dependency for one emitter would put a
`pip install` between an operator and a dry run of the qualification.

Strings are double-quoted with JSON escaping, which YAML 1.2 accepts verbatim,
so no value can be reinterpreted as a boolean, a number, a null or a date — the
Norway problem cannot reach a benchmark set written by this function. Mapping
keys come out in the order they were built in, so two runs over one export
produce byte-identical files, which is what makes a qualification reproducible.
"""

from __future__ import annotations

import json


def dump(value: dict) -> str:
    """One mapping as a complete YAML document, newline-terminated."""
    lines: list = []
    _mapping(value, 0, lines)
    return "\n".join(lines) + "\n"


def _mapping(mapping: dict, indent: int, lines: list) -> None:
    """Emit one block mapping."""
    pad = "  " * indent
    for key, item in mapping.items():
        if isinstance(item, dict) and item:
            lines.append(pad + str(key) + ":")
            _mapping(item, indent + 1, lines)
        elif isinstance(item, list) and item:
            lines.append(pad + str(key) + ":")
            _sequence(item, indent + 1, lines)
        else:
            lines.append(pad + str(key) + ": " + _inline(item))


def _sequence(items: list, indent: int, lines: list) -> None:
    """Emit one block sequence, folding a nested block under its own dash."""
    pad = "  " * indent
    for item in items:
        if isinstance(item, dict) and item:
            nested: list = []
            _mapping(item, indent + 1, nested)
            lines.append(pad + "- " + nested[0].lstrip())
            lines.extend(nested[1:])
        elif isinstance(item, list) and item:
            nested = []
            _sequence(item, indent + 1, nested)
            lines.append(pad + "- " + nested[0].lstrip())
            lines.extend(nested[1:])
        else:
            lines.append(pad + "- " + _inline(item))


def _inline(value) -> str:
    """A value that fits on one line: a scalar, or an empty collection."""
    if isinstance(value, dict):
        return "{}"
    if isinstance(value, list):
        return "[]"
    return _scalar(value)


def _scalar(value) -> str:
    """One scalar, spelled so serde_yaml reads the type that was intended."""
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        return repr(float(value))
    return json.dumps(str(value), ensure_ascii=False)
