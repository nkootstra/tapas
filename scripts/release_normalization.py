#!/usr/bin/env python3
"""Build fail-closed inputs for release-plz version normalization."""

from __future__ import annotations

import argparse
import base64
import json
import pathlib
import re
import sys
from typing import Any


RELEASE_BRANCH = re.compile(r"^release-plz-[A-Za-z0-9._/-]+$")
VERSION = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
SHA = re.compile(r"^[0-9a-f]{40}$")
RELEASE_FILES = {"Cargo.toml", "Cargo.lock", "CHANGELOG.md"}


def select_release_pull_request(
    pull_requests: list[Any], *, repository: str, app_login: str
) -> dict[str, Any]:
    matches = []
    for pull_request in pull_requests:
        if not isinstance(pull_request, dict):
            continue
        user = pull_request.get("user")
        head = pull_request.get("head")
        repo = head.get("repo") if isinstance(head, dict) else None
        branch = head.get("ref") if isinstance(head, dict) else None
        if (
            isinstance(user, dict)
            and isinstance(head, dict)
            and isinstance(repo, dict)
            and isinstance(branch, str)
            and user.get("login") == app_login
            and user.get("type") == "Bot"
            and repo.get("full_name") == repository
            and RELEASE_BRANCH.fullmatch(branch) is not None
        ):
            matches.append(pull_request)
    if len(matches) != 1:
        raise ValueError("expected exactly one open release-plz pull request")
    return matches[0]


def build_commit_request(
    *,
    repository: str,
    branch: str,
    expected_head: str,
    version: str,
    files: list[pathlib.Path],
    root: pathlib.Path,
) -> dict[str, Any]:
    if (
        RELEASE_BRANCH.fullmatch(branch) is None
        or SHA.fullmatch(expected_head) is None
        or VERSION.fullmatch(version) is None
        or not files
    ):
        raise ValueError("release normalization metadata is invalid")
    additions = []
    for path in files:
        relative = path.resolve().relative_to(root.resolve()).as_posix()
        if relative not in RELEASE_FILES:
            raise ValueError(f"normalization changed unexpected file: {relative}")
        additions.append(
            {
                "path": relative,
                "contents": base64.b64encode(path.read_bytes()).decode("ascii"),
            }
        )
    if len({item["path"] for item in additions}) != len(additions):
        raise ValueError("normalization file list contains duplicates")
    return {
        "query": """
          mutation($input: CreateCommitOnBranchInput!) {
            createCommitOnBranch(input: $input) { commit { oid } }
          }
        """,
        "variables": {
            "input": {
                "branch": {
                    "repositoryNameWithOwner": repository,
                    "branchName": branch,
                },
                "expectedHeadOid": expected_head,
                "message": {
                    "headline": f"skip: normalize release version to v{version}"
                },
                "fileChanges": {"additions": additions},
            }
        },
    }


def normalize_body(body: str, old_version: str, new_version: str) -> str:
    if VERSION.fullmatch(old_version) is None or VERSION.fullmatch(new_version) is None:
        raise ValueError("release pull request version is invalid")
    old_version_pattern = re.compile(
        rf"(?<![0-9.]){re.escape(old_version)}(?![0-9.])"
    )
    new_version_pattern = re.compile(
        rf"(?<![0-9.]){re.escape(new_version)}(?![0-9.])"
    )
    if new_version_pattern.search(body) is not None:
        return body
    if old_version_pattern.search(body) is None:
        raise ValueError(f"release pull request body does not contain {old_version}")
    return old_version_pattern.sub(new_version, body)


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    inspect = commands.add_parser("inspect")
    inspect.add_argument("--pulls-json", type=pathlib.Path, required=True)
    inspect.add_argument("--repository", required=True)
    inspect.add_argument("--app-login", required=True)
    inspect.add_argument("--output-json", type=pathlib.Path, required=True)

    mutation = commands.add_parser("mutation")
    mutation.add_argument("--repository", required=True)
    mutation.add_argument("--branch", required=True)
    mutation.add_argument("--expected-head", required=True)
    mutation.add_argument("--version", required=True)
    mutation.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    mutation.add_argument("--output-json", type=pathlib.Path, required=True)
    mutation.add_argument("files", type=pathlib.Path, nargs="+")

    body = commands.add_parser("body")
    body.add_argument("--body-file", type=pathlib.Path, required=True)
    body.add_argument("--old-version", required=True)
    body.add_argument("--new-version", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = argument_parser().parse_args(argv)
    try:
        if args.command == "inspect":
            pull_requests = json.loads(args.pulls_json.read_text(encoding="utf-8"))
            if not isinstance(pull_requests, list):
                raise ValueError("GitHub pull request response must be a list")
            selected = select_release_pull_request(
                pull_requests, repository=args.repository, app_login=args.app_login
            )
            args.output_json.write_text(
                json.dumps(selected, sort_keys=True) + "\n", encoding="utf-8"
            )
        elif args.command == "mutation":
            request = build_commit_request(
                repository=args.repository,
                branch=args.branch,
                expected_head=args.expected_head,
                version=args.version,
                files=args.files,
                root=args.root,
            )
            args.output_json.write_text(
                json.dumps(request, sort_keys=True) + "\n", encoding="utf-8"
            )
        else:
            body = args.body_file.read_text(encoding="utf-8")
            args.body_file.write_text(
                normalize_body(body, args.old_version, args.new_version),
                encoding="utf-8",
            )
        return 0
    except (ValueError, OSError, json.JSONDecodeError) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
