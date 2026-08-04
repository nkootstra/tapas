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

ROOT = pathlib.Path(__file__).resolve().parents[1]
CATALOG = ROOT / "src" / "catalog.rs"
FILTERS_DIR = ROOT / "src" / "filters"
GIT = FILTERS_DIR / "git.rs"
TESTS_DIR = ROOT / "tests"

# Filter families and the Rust test file expected to cover them.
FILTER_FAMILIES: dict[str, tuple[str, ...]] = {
    "build": ("make", "ninja", "cargo", "go", "zig", "npm", "pnpm", "yarn", "bun", "dotnet", "gradle", "gradlew", "mvn", "mvnw", "next", "webpack", "turbo", "swift", "xcodebuild", "uv", "uvx", "npx", "poetry"),
    "data": ("aws", "jq", "pup", "acli", "cat", "gh", "sqlite3", "brew", "df", "lsof", "ps", "psql", "systemctl", "docker", "docker-compose", "kubectl"),
    "diagnostics": ("mypy", "ruff", "eslint", "biome", "pre-commit", "prettier", "terraform", "tofu"),
    "infra": ("curl", "docker", "docker-compose", "kubectl", "gh", "acli"),
    "listing": ("find", "tree", "ls", "du", "wc", "env", "rg"),
    "package": ("npm", "pnpm", "yarn", "bun", "composer", "pip", "pip3"),
    "test_tools": ("pytest", "jest", "vitest", "mocha", "tsc", "cargo", "go", "node"),
}

# Transparent runners dispatch to the inner command rather than being filtered
# themselves, so they are not required to have a direct filter family.
TRANSPARENT_RUNNERS = {"bunx", "npx", "uvx"}

# Shells and env wrappers are content-redispatched or left to the caller.
EXEMPT_FROM_FILTER_FAMILY = {"bash", "sh", "zsh", "env", "head", "tail"}


def parse_const(source: str, name: str) -> list[str]:
    match = re.search(rf"pub const {name}:\s*&\s*\[\s*&\s*str\]\s*=\s*&\[(.*?)\];", source, re.S)
    if not match:
        raise RuntimeError(f"cannot parse catalog constant {name}")
    return re.findall(r'"([^"]+)"', match.group(1))


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
) -> list[str]:
    errors: list[str] = []

    auto_wrap_set = set(auto_wrap)
    wrapper_set = set(wrapper)

    missing_wrapper = auto_wrap_set - wrapper_set
    if missing_wrapper:
        errors.append(
            f"AUTO_WRAP_COMMANDS missing from WRAPPER_COMMANDS: {', '.join(sorted(missing_wrapper))}"
        )

    # Parse the actual filter implementations.
    handled_by_family: dict[str, set[str]] = {}
    for family, _ in FILTER_FAMILIES.items():
        handled = set()
        source_file = FILTERS_DIR / f"{family}.rs"
        if not source_file.is_file():
            errors.append(f"filter family source missing: {source_file.name}")
            continue
        source = source_file.read_text(encoding="utf-8")
        match = re.search(r"pub\(crate\) fn handles_argv.*?\n}", source, re.S)
        if match:
            handled.update(
                token.strip('"')
                for token in re.findall(r'b"([a-z0-9.-]+)"', match.group(0))
            )
        handled_by_family[family] = handled

    handled_anywhere = set().union(*handled_by_family.values()) if handled_by_family else set()

    for family, expected in FILTER_FAMILIES.items():
        missing = set(expected) - handled_by_family.get(family, set())
        if missing:
            errors.append(
                f"{family} catalog ownership is missing from handles_argv: {', '.join(sorted(missing))}"
            )

    # Every auto-wrap command must be handled by a filter family, be a
    # transparent runner, or be exempt.
    uncovered = (
        auto_wrap_set
        - handled_anywhere
        - TRANSPARENT_RUNNERS
        - EXEMPT_FROM_FILTER_FAMILY
        - {"git"}  # git is dispatched by git.rs directly.
    )
    if uncovered:
        errors.append(
            f"auto-wrap commands with no filter family: {', '.join(sorted(uncovered))}"
        )

    transparent_commands = {
        runner.split(maxsplit=1)[0] for runner in ("bunx", "npx", "uvx")
    }
    wrapper_uncovered = wrapper_set - handled_anywhere - transparent_commands - EXEMPT_FROM_FILTER_FAMILY - {"git"}
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
    for family in FILTER_FAMILIES:
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
    except RuntimeError as exc:
        print(f"catalog audit failed: {exc}", file=sys.stderr)
        return 1
    errors = check_catalog(
        auto_wrap=auto_wrap,
        wrapper=wrapper,
        git_subcommands=git_subcommands,
    )
    if errors:
        print("catalog audit failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        f"catalog audit passed: {len(auto_wrap)} auto-wrap, "
        f"{len(git_subcommands)} git subcommands, {len(FILTER_FAMILIES)} filter families"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
