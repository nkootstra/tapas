#!/usr/bin/env python3
"""Independently audit the tapas-owned command catalog for internal consistency.

The catalog is tapas-owned: smll is archived and no longer acts as a coverage
authority.  This audit verifies that every catalog entry is backed by a real
filter implementation, every git subcommand has a dispatch arm, and every
filter family has regression tests, so agent-readable output stays covered.
"""

from __future__ import annotations

import argparse
import json
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
COVERAGE = TESTS_DIR / "regression" / "coverage.json"
CASES = TESTS_DIR / "regression" / "cases.json"


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


def parse_stream_filters(source: str) -> dict[str, str]:
    match = re.search(
        r"const STREAM_FILTERS:\s*&\[StreamFilterSpec\]\s*=\s*&\[(.*?)\n\];",
        source,
        re.S,
    )
    if not match:
        raise ValueError("stream filter registry not found")
    filters: dict[str, str] = {}
    entries = re.findall(
        r"StreamFilterSpec\s*\{(.*?)\n\s*\},", match.group(1), re.S
    )
    for entry in entries:
        name = re.search(r'name:\s*"([a-z-]+)"', entry)
        handler = re.search(
            r"handles:\s*crate::filters::([a-z_]+)::handles_argv",
            entry,
        )
        if not name or not handler:
            raise ValueError("stream filter registry entry is missing a name or handler")
        filters[name.group(1).replace("-", "_")] = handler.group(1)
    return filters


def handler_catalog_constants(source: str) -> set[str]:
    signature = re.search(r"pub\(crate\)\s+fn handles_argv\b", source)
    if not signature:
        return set()
    body_start = source.find("{", signature.end())
    if body_start == -1:
        return set()

    depth = 0
    for index in range(body_start, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                body = source[body_start + 1 : index]
                return set(
                    re.findall(
                        r"crate::catalog::([A-Z][A-Z0-9_]*_FILTER_COMMANDS)\b",
                        body,
                    )
                )
    return set()


def check_handler_wiring(family: str, source: str) -> str | None:
    expected = f"{family.upper()}_FILTER_COMMANDS"
    if expected not in handler_catalog_constants(source):
        return f"filter family {family} handles_argv does not reference {expected}"
    return None


def check_behavior_coverage(
    *,
    auto_wrap: list[str],
    git_subcommands: list[str],
    compact_routes: list[str],
    exact_policies: list[str],
    inherited_policies: list[str],
    contract: dict[str, dict[str, str]],
    regression_tags: set[str],
) -> list[str]:
    """Require a real test reference or an existing regression-case tag."""

    errors: list[str] = []

    def covered(section: str, name: str, tag_prefix: str) -> bool:
        return (
            name in contract.get(section, {})
            or f"{tag_prefix}:{name}" in regression_tags
        )

    compact_ids = [route.split(":", 1)[-1] for route in compact_routes]
    missing = [name for name in compact_ids if not covered("compact_routes", name, "compact_route")]
    if missing:
        errors.append(f"compact routes without behavior coverage: {', '.join(sorted(missing))}")
    missing = [name for name in exact_policies if not covered("exact_output_policies", name, "exact_output_bypass")]
    if missing:
        errors.append(f"exact-output policies without behavior coverage: {', '.join(sorted(missing))}")
    missing = [name for name in inherited_policies if not covered("inherited_stream_policies", name, "stream_watch_policy")]
    if missing:
        errors.append(f"inherited/stream policies without behavior coverage: {', '.join(sorted(missing))}")

    auto_contract = contract.get("auto_wrap_commands", {})
    auto_wildcard = auto_contract.get("*")
    missing = [name for name in auto_wrap if name not in auto_contract and not auto_wildcard]
    if missing:
        errors.append(f"auto-wrap commands without behavior coverage: {', '.join(sorted(missing))}")
    missing = [name for name in git_subcommands if not covered("git_subcommands", name, "git_subcommand")]
    if missing:
        errors.append(f"git subcommands without behavior coverage: {', '.join(sorted(missing))}")

    source_cache: dict[pathlib.Path, str] = {}
    test_cache: dict[pathlib.Path, set[str]] = {}
    for section, references in contract.items():
        if not isinstance(references, dict):
            continue
        for name, reference in references.items():
            if not isinstance(reference, str) or "::" not in reference:
                errors.append(f"invalid behavior coverage reference for {section}.{name}: {reference!r}")
                continue
            relative, test_name = reference.split("::", 1)
            path = ROOT / relative
            if path not in source_cache:
                source = path.read_text(encoding="utf-8") if path.is_file() else ""
                source_cache[path] = source
                test_cache[path] = set(
                    re.findall(r"#\[test\]\s*fn\s+([A-Za-z_][A-Za-z0-9_]*)\b", source)
                )
            source = source_cache[path]
            if test_name not in test_cache[path]:
                errors.append(f"behavior coverage test not found for {section}.{name}: {reference}")
            if section == "auto_wrap_commands" and name == "*" and (
                "AUTO_WRAP_COMMANDS" not in source or '"--rewrite"' not in source
            ):
                errors.append(f"auto-wrap wildcard coverage must iterate AUTO_WRAP_COMMANDS and --rewrite: {reference}")
    return errors


def load_regression_tags(path: pathlib.Path) -> set[str]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    return {tag for case in payload.get("cases", []) for tag in case.get("covers", [])}


def check_catalog(
    *,
    auto_wrap: list[str],
    wrapper: list[str],
    git_subcommands: list[str],
    transparent_runners: list[str],
    filter_families: dict[str, set[str]],
    filter_family_exemptions: set[str],
    stream_filters: dict[str, str],
) -> list[str]:
    errors: list[str] = []
    stream_filter_names = set(stream_filters)

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

    for family, handler_family in stream_filters.items():
        if handler_family != family:
            errors.append(
                f"stream filter registry handler mismatch for {family}: "
                f"expected crate::filters::{family}::handles_argv, "
                f"found crate::filters::{handler_family}::handles_argv"
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
        else:
            wiring_error = check_handler_wiring(
                family, source_file.read_text(encoding="utf-8")
            )
            if wiring_error:
                errors.append(wiring_error)
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
        compact_routes = parse_const(source, "COMPACT_ROUTES")
        exact_policies = parse_const(source, "EXACT_OUTPUT_BYPASSES")
        inherited_policies = parse_const(source, "STREAM_WATCH_POLICIES")
        filter_families = parse_filter_families(source)
        filter_family_exemptions = parse_byte_const(source, "FILTER_FAMILY_EXEMPTIONS")
        stream_filters = parse_stream_filters(PROCESS.read_text(encoding="utf-8"))
        contract = json.loads(COVERAGE.read_text(encoding="utf-8"))
        regression_tags = load_regression_tags(CASES)
    except (RuntimeError, ValueError, OSError, json.JSONDecodeError) as exc:
        print(f"catalog audit failed: {exc}", file=sys.stderr)
        return 1
    errors = check_catalog(
        auto_wrap=auto_wrap,
        wrapper=wrapper,
        git_subcommands=git_subcommands,
        transparent_runners=transparent_runners,
        filter_families=filter_families,
        filter_family_exemptions=filter_family_exemptions,
        stream_filters=stream_filters,
    )
    errors.extend(
        check_behavior_coverage(
            auto_wrap=auto_wrap,
            git_subcommands=git_subcommands,
            compact_routes=compact_routes,
            exact_policies=exact_policies,
            inherited_policies=inherited_policies,
            contract=contract,
            regression_tags=regression_tags,
        )
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
