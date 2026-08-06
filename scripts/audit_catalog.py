#!/usr/bin/env python3
"""Independently audit the tapas-owned command catalog for internal consistency.

The catalog is tapas-owned: smll is archived and no longer acts as a coverage
authority.  This audit verifies that every catalog entry is backed by a real
filter implementation, every git subcommand has a dispatch arm, and every
filter family has regression tests, so agent-readable output stays covered.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from typing import Any

from catalog_source import parse_byte_const, parse_filter_families, parse_string_const

ROOT = pathlib.Path(__file__).resolve().parents[1]
CATALOG = ROOT / "src" / "catalog.rs"
FILTERS_DIR = ROOT / "src" / "filters"
GIT = FILTERS_DIR / "git.rs"
PROCESS = ROOT / "src" / "process" / "mod.rs"
TESTS_DIR = ROOT / "tests"


def parse_const(source: str, name: str) -> list[str]:
    try:
        return parse_string_const(source, name)
    except ValueError as error:
        raise RuntimeError(f"cannot parse catalog constant {name}") from error


def parse_git_dispatch(source: str) -> set[str]:
    match = re.search(r"match argv\[1\] \{(.*?)\n    }\n}\n\n/// Apply Git", source, re.S)
    if not match:
        return set()
    arms = re.findall(
        r"((?:b\"[a-z-]+\"\s*(?:\|\s*)?)+)\s*=>",
        match.group(1),
    )
    return {
        token
        for arm in arms
        for token in re.findall(r'b\"([a-z-]+)\"', arm)
    }


def parse_stream_filter_names(source: str) -> set[str]:
    match = re.search(
        r"const STREAM_FILTERS:\s*&\[StreamFilterSpec\]\s*=\s*&\[(.*?)\n\];",
        source,
        re.S,
    )
    if not match:
        raise ValueError("stream filter registry not found")
    return {
        name.replace("-", "_")
        for name in re.findall(r'name:\s*"([a-z-]+)"', match.group(1))
    }


def check_catalog(
    *,
    auto_wrap: list[str],
    wrapper: list[str],
    git_subcommands: list[str],
    transparent_runners: list[str],
    filter_families: dict[str, set[str]],
    filter_family_exemptions: set[str],
    stream_filter_names: set[str],
) -> list[str]:
    errors: list[str] = []

    auto_wrap_set = set(auto_wrap)
    wrapper_set = set(wrapper)

    missing_wrapper = auto_wrap_set - wrapper_set
    if missing_wrapper:
        errors.append(
            f"AUTO_WRAP_COMMANDS missing from WRAPPER_COMMANDS: {', '.join(sorted(missing_wrapper))}"
        )

    missing_families = stream_filter_names - filter_families.keys()
    if missing_families:
        errors.append(
            f"filter family catalogs missing: {', '.join(sorted(missing_families))}"
        )
    unexpected_families = filter_families.keys() - stream_filter_names
    if unexpected_families:
        errors.append(
            f"unexpected filter family catalogs: {', '.join(sorted(unexpected_families))}"
        )

    handled_anywhere = (
        set().union(*filter_families.values()) if filter_families else set()
    )

    # Every auto-wrap command must be handled by a filter family, be a
    # transparent runner, or be exempt.
    uncovered = (
        auto_wrap_set
        - handled_anywhere
        - filter_family_exemptions
    )
    if uncovered:
        errors.append(
            f"auto-wrap commands with no filter family: {', '.join(sorted(uncovered))}"
        )

    transparent_commands = {runner.split(maxsplit=1)[0] for runner in transparent_runners}
    wrapper_uncovered = (
        wrapper_set - handled_anywhere - transparent_commands - filter_family_exemptions
    )
    if wrapper_uncovered:
        errors.append(
            f"wrapper commands with no filter family: {', '.join(sorted(wrapper_uncovered))}"
        )

    # Git subcommands must all have dispatch arms in git.rs.
    git_source = GIT.read_text(encoding="utf-8") if GIT.is_file() else ""
    git_arms = parse_git_dispatch(git_source)
    missing_arms = set(git_subcommands) - git_arms
    if missing_arms:
        errors.append(
            f"git subcommands without a dispatch arm in git.rs: {', '.join(sorted(missing_arms))}"
        )

    # Every filter family must have regression tests and fixtures.
    for family in stream_filter_names:
        source_file = FILTERS_DIR / f"{family}.rs"
        if not source_file.is_file():
            errors.append(f"filter family source missing: {source_file.name}")
        test_file = TESTS_DIR / f"filters_{family}.rs"
        if not test_file.is_file():
            errors.append(f"no regression test file for {family}: {test_file.name}")
            continue
        test_source = test_file.read_text(encoding="utf-8")
        test_count = len(re.findall(r"#\[test\]", test_source))
        if test_count == 0:
            errors.append(f"filter family {family} has no #[test] cases")

    return errors


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=pathlib.Path, default=CATALOG)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    errors: list[str] = []
    if not args.catalog.is_file():
        print(f"catalog not found: {args.catalog}", file=sys.stderr)
        return 2
    source = args.catalog.read_text(encoding="utf-8")
    try:
        auto_wrap = parse_const(source, "AUTO_WRAP_COMMANDS")
        wrapper = parse_const(source, "WRAPPER_COMMANDS")
        git_subcommands = parse_const(source, "GIT_SUBCOMMANDS")
        transparent_runners = parse_const(source, "TRANSPARENT_RUNNERS")
        filter_families = parse_filter_families(source)
        filter_family_exemptions = parse_byte_const(source, "FILTER_FAMILY_EXEMPTIONS")
        stream_filter_names = parse_stream_filter_names(PROCESS.read_text(encoding="utf-8"))
    except (RuntimeError, ValueError) as exc:
        print(f"catalog audit failed: {exc}", file=sys.stderr)
        return 1
    errors = check_catalog(
        auto_wrap=auto_wrap,
        wrapper=wrapper,
        git_subcommands=git_subcommands,
        transparent_runners=transparent_runners,
        filter_families=filter_families,
        filter_family_exemptions=filter_family_exemptions,
        stream_filter_names=stream_filter_names,
    )
    if errors:
        print("catalog audit failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        f"catalog audit passed: {len(auto_wrap)} auto-wrap, "
        f"{len(git_subcommands)} git subcommands, {len(filter_families)} filter families"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
