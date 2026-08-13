#!/usr/bin/env python3
"""Build fail-closed inputs for release-plz version normalization."""

from __future__ import annotations

import argparse
import base64
import json
import pathlib
import re
import sys
from dataclasses import dataclass
from typing import Any

from release_contract import RELEASE_BRANCH, RELEASE_FILES, RELEASE_TITLE, SHA


VERSION = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")


@dataclass(frozen=True)
class AutoMergeCandidate:
    number: int
    head_sha: str
    title: str
    disable_existing: bool


def select_release_pull_request(
    pull_requests: list[Any],
    *,
    repository: str,
    app_login: str,
    allow_none: bool = False,
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
    if not matches and allow_none:
        return {}
    if len(matches) != 1:
        raise ValueError("expected exactly one open release-plz pull request")
    return matches[0]


def validate_auto_merge_candidate(
    pull_request: dict[str, Any],
    files: list[Any],
    *,
    repository: str,
    app_login: str,
) -> AutoMergeCandidate:
    base = pull_request.get("base")
    head = pull_request.get("head")
    user = pull_request.get("user")
    repo = head.get("repo") if isinstance(head, dict) else None
    auto_merge = pull_request.get("auto_merge")
    number = pull_request.get("number")
    title = pull_request.get("title")
    head_sha = head.get("sha") if isinstance(head, dict) else None
    filenames = {
        item.get("filename") for item in files if isinstance(item, dict)
    }
    valid_auto_merge = auto_merge is None
    if isinstance(auto_merge, dict):
        enabled_by = auto_merge.get("enabled_by")
        valid_auto_merge = (
            auto_merge.get("merge_method") == "squash"
            and isinstance(enabled_by, dict)
            and enabled_by.get("login") == app_login
            and enabled_by.get("type") == "Bot"
        )
    if (
        not isinstance(base, dict)
        or not isinstance(head, dict)
        or not isinstance(user, dict)
        or not isinstance(repo, dict)
        or not isinstance(number, int)
        or number < 1
        or not isinstance(title, str)
        or RELEASE_TITLE.fullmatch(title) is None
        or pull_request.get("state") != "open"
        or pull_request.get("draft") is not False
        or base.get("ref") != "main"
        or user.get("login") != app_login
        or user.get("type") != "Bot"
        or repo.get("full_name") != repository
        or RELEASE_BRANCH.fullmatch(head.get("ref", "")) is None
        or not isinstance(head_sha, str)
        or SHA.fullmatch(head_sha) is None
        or filenames != RELEASE_FILES
        or len(files) != len(RELEASE_FILES)
        or not valid_auto_merge
    ):
        raise ValueError("release pull request is not safe to auto-merge")
    return AutoMergeCandidate(
        number=number,
        head_sha=head_sha,
        title=title,
        disable_existing=auto_merge is not None,
    )


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
    inspect.add_argument("--allow-none", action="store_true")

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

    auto_merge = commands.add_parser("auto-merge")
    auto_merge.add_argument("--pr-json", type=pathlib.Path, required=True)
    auto_merge.add_argument("--files-json", type=pathlib.Path, required=True)
    auto_merge.add_argument("--repository", required=True)
    auto_merge.add_argument("--app-login", required=True)
    auto_merge.add_argument("--output-json", type=pathlib.Path, required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = argument_parser().parse_args(argv)
    try:
        if args.command == "inspect":
            pull_requests = json.loads(args.pulls_json.read_text(encoding="utf-8"))
            if not isinstance(pull_requests, list):
                raise ValueError("GitHub pull request response must be a list")
            selected = select_release_pull_request(
                pull_requests,
                repository=args.repository,
                app_login=args.app_login,
                allow_none=args.allow_none,
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
        elif args.command == "body":
            body = args.body_file.read_text(encoding="utf-8")
            args.body_file.write_text(
                normalize_body(body, args.old_version, args.new_version),
                encoding="utf-8",
            )
        elif args.command == "auto-merge":
            pull_request = json.loads(args.pr_json.read_text(encoding="utf-8"))
            files = json.loads(args.files_json.read_text(encoding="utf-8"))
            if not isinstance(pull_request, dict) or not isinstance(files, list):
                raise ValueError("GitHub API responses have unexpected JSON shapes")
            candidate = validate_auto_merge_candidate(
                pull_request,
                files,
                repository=args.repository,
                app_login=args.app_login,
            )
            args.output_json.write_text(
                json.dumps(
                    {
                        "disable_existing": candidate.disable_existing,
                        "head_sha": candidate.head_sha,
                        "number": candidate.number,
                        "title": candidate.title,
                    },
                    sort_keys=True,
                )
                + "\n",
                encoding="utf-8",
            )
        else:
            raise ValueError("unknown release normalization command")
        return 0
    except (ValueError, OSError, json.JSONDecodeError) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
