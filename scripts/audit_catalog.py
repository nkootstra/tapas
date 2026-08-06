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

from catalog_source import parse_filter_families, parse_string_const

ROOT = pathlib.Path(__file__).resolve().parents[1]
CATALOG = ROOT / "src" / "catalog.rs"
FILTERS_DIR = ROOT / "src" / "filters"
GIT = FILTERS_DIR / "git.rs"
TESTS_DIR = ROOT / "tests"

EXPECTED_FILTER_FAMILIES = {
    "build",
    "data",
    "diagnostics",
    "git",
    "infra",
    "listing",
    "package",
    "test_tools",
}

# Shells and env wrappers are content-redispatched or left to the caller.
EXEMPT_FROM_FILTER_FAMILY = {"bash", "sh", "zsh", "env", "head", "tail"}


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


def check_catalog(
    *,
    auto_wrap: list[str],
    wrapper: list[str],
    git_subcommands: list[str],
    transparent_runners: list[str],
    filter_families: dict[str, set[str]],
) -> list[str]:
    errors: list[str] = []

    auto_wrap_set = set(auto_wrap)
    wrapper_set = set(wrapper)

    missing_wrapper = auto_wrap_set - wrapper_set
    if missing_wrapper:
        errors.append(
            f"AUTO_WRAP_COMMANDS missing from WRAPPER_COMMANDS: {', '.join(sorted(missing_wrapper))}"
        )

    missing_families = EXPECTED_FILTER_FAMILIES - filter_families.keys()
    if missing_families:
        errors.append(
            f"filter family catalogs missing: {', '.join(sorted(missing_families))}"
        )
    unexpected_families = filter_families.keys() - EXPECTED_FILTER_FAMILIES
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
        - EXEMPT_FROM_FILTER_FAMILY
    )
    if uncovered:
        errors.append(
            f"auto-wrap commands with no filter family: {', '.join(sorted(uncovered))}"
        )

    transparent_commands = {runner.split(maxsplit=1)[0] for runner in transparent_runners}
    wrapper_uncovered = (
        wrapper_set - handled_anywhere - transparent_commands - EXEMPT_FROM_FILTER_FAMILY
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
    for family in EXPECTED_FILTER_FAMILIES:
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
    except (RuntimeError, ValueError) as exc:
        print(f"catalog audit failed: {exc}", file=sys.stderr)
        return 1
    errors = check_catalog(
        auto_wrap=auto_wrap,
        wrapper=wrapper,
        git_subcommands=git_subcommands,
        transparent_runners=transparent_runners,
        filter_families=filter_families,
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
