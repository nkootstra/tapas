"""Shared invariants for trusted release pull requests."""

from __future__ import annotations

import re


RELEASE_TITLE = re.compile(
    r"^skip: prepare v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$"
)
RELEASE_BRANCH = re.compile(r"^release-plz-[A-Za-z0-9._/-]+$")
SHA = re.compile(r"^[0-9a-f]{40}$")
RELEASE_FILES = {"Cargo.toml", "Cargo.lock", "CHANGELOG.md"}
