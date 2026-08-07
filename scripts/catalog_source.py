"""Parse the Rust-owned Tapas command catalog without importing Rust code."""

from __future__ import annotations

import re


def parse_string_const(source: str, name: str) -> list[str]:
    match = re.search(
        rf"pub const {name}:\s*&\s*\[\s*&\s*str\]\s*=\s*&\[(.*?)\];",
        source,
        re.S,
    )
    if not match:
        raise ValueError(f"catalog constant not found: {name}")
    return re.findall(r'"([^"\\]+)"', match.group(1))


def parse_catalog(source: str) -> dict[str, set[str]]:
    return {
        name: set(parse_string_const(source, name))
        for name in (
            "AUTO_WRAP_COMMANDS",
            "WRAPPER_COMMANDS",
            "GIT_SUBCOMMANDS",
            "TRANSPARENT_RUNNERS",
        )
    }


def parse_byte_const(source: str, name: str) -> set[str]:
    match = re.search(
        rf"pub\(crate\) const {name}:\s*&\s*\[\s*&\s*\[u8\]\]\s*=\s*&\[(.*?)\];",
        source,
        re.S,
    )
    if not match:
        raise ValueError(f"catalog byte constant not found: {name}")
    return set(re.findall(r'b"([^"\\]+)"', match.group(1)))


def parse_filter_families(source: str) -> dict[str, set[str]]:
    pattern = re.compile(
        r"pub\(crate\) const ([A-Z_]+)_FILTER_COMMANDS:\s*"
        r"&\s*\[\s*&\s*\[u8\]\]\s*=\s*&\[(.*?)\];",
        re.S,
    )
    families: dict[str, set[str]] = {}
    for match in pattern.finditer(source):
        family = match.group(1).lower()
        if family in families:
            raise ValueError(f"duplicate filter family catalog: {family}")
        families[family] = set(re.findall(r'b"([^"\\]+)"', match.group(2)))
    if not families:
        raise ValueError("filter family catalogs not found")
    return families
