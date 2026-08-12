#!/usr/bin/env python3
"""Validate release-intent titles and calculate literal SemVer bumps."""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
import tomllib
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


def repository_version(manifest_path: pathlib.Path, lockfile_path: pathlib.Path) -> str:
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    lockfile = tomllib.loads(lockfile_path.read_text(encoding="utf-8"))
    package = manifest.get("package")
    if not isinstance(package, dict):
        raise ValueError("Cargo.toml must contain a package table")
    name = package.get("name")
    version = package.get("version")
    if (
        not isinstance(name, str)
        or not isinstance(version, str)
        or version.startswith("v")
        or VERSION_PATTERN.fullmatch(version) is None
    ):
        raise ValueError("Cargo.toml package name or version is invalid")

    packages = lockfile.get("package")
    if not isinstance(packages, list):
        raise ValueError("Cargo.lock must contain package entries")
    matching_versions = [
        item.get("version")
        for item in packages
        if isinstance(item, dict) and item.get("name") == name
    ]
    if matching_versions != [version]:
        raise ValueError(
            "Cargo.toml and Cargo.lock package versions must match exactly"
        )
    return version


def pending_release_version(
    current: str, repository: str, subjects: Iterable[str]
) -> str | None:
    current_match = VERSION_PATTERN.fullmatch(current)
    repository_match = VERSION_PATTERN.fullmatch(repository)
    if (
        current_match is None
        or repository.startswith("v")
        or repository_match is None
    ):
        raise ValueError("release versions must use vX.Y.Z or X.Y.Z")

    current_version = ".".join(current_match.groups())
    bump = select_bump(subjects)
    if bump is None:
        if repository == current_version:
            return None
        raise ValueError(
            f"repository version {repository} does not match current "
            f"{current_version} when no release bump is pending"
        )
    desired = next_version(current, bump)
    if repository == current_version:
        return desired
    if repository == desired:
        return None
    raise ValueError(
        f"repository version {repository} is neither current {current_version} "
        f"nor desired {desired}"
    )


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    title = commands.add_parser("validate-title")
    title.add_argument("title")

    version = commands.add_parser("next-version")
    version.add_argument("--current", required=True)
    version.add_argument("--manifest", type=pathlib.Path, required=True)
    version.add_argument("--lockfile", type=pathlib.Path, required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = argument_parser().parse_args(argv)
    try:
        if args.command == "validate-title":
            validate_title(args.title)
            return 0

        prepared = repository_version(args.manifest, args.lockfile)
        pending = pending_release_version(
            args.current,
            prepared,
            (line.rstrip("\r\n") for line in sys.stdin),
        )
        if pending is not None:
            print(pending)
        return 0
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
