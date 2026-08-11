#!/usr/bin/env python3
"""Validate release-intent titles and calculate literal SemVer bumps."""

from __future__ import annotations

import argparse
import re
import sys
from collections.abc import Iterable


TITLE_PATTERN = re.compile(r"^(major|minor|patch|skip): [^\r\n]*\S[^\r\n]*$")
VERSION_PATTERN = re.compile(
    r"^v?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$"
)
RELEASE_RANK = {"patch": 1, "minor": 2, "major": 3}


def validate_title(title: str) -> None:
    if TITLE_PATTERN.fullmatch(title) is None:
        raise ValueError(
            "pull request titles must start with major:, minor:, patch:, or skip: "
            "followed by a description"
        )


def select_bump(subjects: Iterable[str]) -> str | None:
    selected: str | None = None
    for subject in subjects:
        prefix, separator, _ = subject.partition(":")
        if not separator or prefix not in RELEASE_RANK:
            continue
        if selected is None or RELEASE_RANK[prefix] > RELEASE_RANK[selected]:
            selected = prefix
    return selected


def next_version(current: str, bump: str) -> str:
    match = VERSION_PATTERN.fullmatch(current)
    if match is None:
        raise ValueError(f"current version must use vX.Y.Z or X.Y.Z: {current}")
    if bump not in RELEASE_RANK:
        raise ValueError(f"unsupported release bump: {bump}")

    major, minor, patch = (int(part) for part in match.groups())
    if bump == "major":
        major, minor, patch = major + 1, 0, 0
    elif bump == "minor":
        minor, patch = minor + 1, 0
    else:
        patch += 1
    return f"{major}.{minor}.{patch}"


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    title = commands.add_parser("validate-title")
    title.add_argument("title")

    version = commands.add_parser("next-version")
    version.add_argument("--current", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = argument_parser().parse_args(argv)
    try:
        if args.command == "validate-title":
            validate_title(args.title)
            return 0

        bump = select_bump(line.rstrip("\r\n") for line in sys.stdin)
        if bump is not None:
            print(next_version(args.current, bump))
        return 0
    except ValueError as error:
        print(error, file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
