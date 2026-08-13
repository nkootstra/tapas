#!/usr/bin/env python3
"""Validate and create a trusted signed release tag."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from typing import Any

from release_contract import RELEASE_BRANCH, RELEASE_FILES, RELEASE_TITLE, SHA
import release_policy


@dataclass(frozen=True)
class Candidate:
    version: str
    merge_sha: str

    @property
    def tag(self) -> str:
        return f"v{self.version}"


def validate_changed_files(files: list[dict[str, Any]]) -> None:
    if {item.get("filename") for item in files} != RELEASE_FILES or len(files) != 3:
        raise ValueError("release pull request must change exactly the release files")


def validate_signing_key(
    signing_key: pathlib.Path, trusted_signers: pathlib.Path
) -> tuple[str, str]:
    result = subprocess.run(
        ["/usr/bin/ssh-keygen", "-y", "-f", str(signing_key)],
        env={**os.environ, "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"},
        check=False,
        capture_output=True,
        text=True,
    )
    public_key = result.stdout.strip()
    if result.returncode != 0 or not public_key.startswith("ssh-ed25519 "):
        raise ValueError("release signing key must be a readable Ed25519 private key")

    key_parts = public_key.split()
    allowed: dict[str, str] = {}
    for line in trusted_signers.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        match = re.search(r"ssh-ed25519\s+[A-Za-z0-9+/]+={0,2}", line)
        if match is not None:
            allowed[match.group(0)] = line.split()[0].split(",", 1)[0]
    normalized = " ".join(key_parts[:2])
    if normalized not in allowed:
        raise ValueError("release signing key is not in the trusted signers file")
    return normalized, allowed[normalized]


def _verify_tag(
    ref: str, candidate: Candidate, trusted_signers: pathlib.Path, cwd: pathlib.Path
) -> None:
    if _git(cwd, "cat-file", "-t", ref) != "tag":
        raise ValueError("release tag must be annotated")
    _git(
        cwd,
        "-c",
        "gpg.format=ssh",
        "-c",
        "gpg.ssh.program=/usr/bin/ssh-keygen",
        "-c",
        f"gpg.ssh.allowedSignersFile={trusted_signers.resolve()}",
        "verify-tag",
        ref,
    )
    if _git(cwd, "rev-list", "-n", "1", ref) != candidate.merge_sha:
        raise ValueError("release tag targets an unexpected commit")


def _remote_tag(
    candidate: Candidate,
    *,
    remote: str,
    trusted_signers: pathlib.Path,
    cwd: pathlib.Path,
) -> bool:
    output = _git(
        cwd,
        "ls-remote",
        "--tags",
        remote,
        f"refs/tags/{candidate.tag}",
        f"refs/tags/{candidate.tag}^{{}}",
    )
    if not output:
        return False
    refs = {line.split()[1]: line.split()[0] for line in output.splitlines()}
    tag_ref = f"refs/tags/{candidate.tag}"
    peeled_ref = f"{tag_ref}^{{}}"
    if set(refs) != {tag_ref, peeled_ref} or refs[peeled_ref] != candidate.merge_sha:
        raise ValueError("existing remote release tag conflicts with the candidate")

    validation_ref = f"refs/tapas-release-validation/{refs[tag_ref]}"
    _git(cwd, "fetch", "--no-tags", remote, f"{tag_ref}:{validation_ref}")
    if _git(cwd, "rev-parse", validation_ref) != refs[tag_ref]:
        raise ValueError("fetched release tag differs from the remote tag")
    _verify_tag(validation_ref, candidate, trusted_signers, cwd)
    return True


def create_or_verify_tag(
    candidate: Candidate,
    *,
    signing_key: pathlib.Path,
    trusted_signers: pathlib.Path,
    remote: str,
    cwd: pathlib.Path,
) -> str:
    _, identity = validate_signing_key(signing_key, trusted_signers)
    if _remote_tag(
        candidate, remote=remote, trusted_signers=trusted_signers, cwd=cwd
    ):
        return "existing"

    local_ref = f"refs/tags/{candidate.tag}"
    local_exists = subprocess.run(
        ["git", "show-ref", "--verify", "--quiet", local_ref],
        cwd=cwd,
        env={**os.environ, "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"},
        check=False,
    ).returncode == 0
    if local_exists:
        _verify_tag(local_ref, candidate, trusted_signers, cwd)
    else:
        _git(
            cwd,
            "-c",
            "user.name=Tapas Release",
            "-c",
            f"user.email={identity}",
            "-c",
            "gpg.format=ssh",
            "-c",
            "gpg.ssh.program=/usr/bin/ssh-keygen",
            "-c",
            f"user.signingkey={signing_key.resolve()}",
            "tag",
            "--sign",
            "--annotate",
            candidate.tag,
            candidate.merge_sha,
            "--message",
            f"Release {candidate.tag}",
        )
        _verify_tag(local_ref, candidate, trusted_signers, cwd)

    _git(cwd, "push", remote, f"{local_ref}:{local_ref}")
    if not _remote_tag(
        candidate, remote=remote, trusted_signers=trusted_signers, cwd=cwd
    ):
        raise ValueError("release tag was not created on the remote")
    return "created"


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    validate = commands.add_parser("validate")
    validate.add_argument("--pr-json", type=pathlib.Path, required=True)
    validate.add_argument("--files-json", type=pathlib.Path, required=True)
    validate.add_argument("--expected-pr-number", type=int, required=True)
    validate.add_argument("--repository", required=True)
    validate.add_argument("--app-login", required=True)
    validate.add_argument("--workflow-sha", required=True)
    validate.add_argument("--candidate-json", type=pathlib.Path, required=True)

    tag = commands.add_parser("tag")
    tag.add_argument("--candidate-json", type=pathlib.Path, required=True)
    tag.add_argument("--signing-key", type=pathlib.Path, required=True)
    tag.add_argument("--trusted-signers", type=pathlib.Path, required=True)
    tag.add_argument("--remote", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = argument_parser().parse_args(argv)
    try:
        if args.command == "validate":
            pull_request = json.loads(args.pr_json.read_text(encoding="utf-8"))
            files = json.loads(args.files_json.read_text(encoding="utf-8"))
            if not isinstance(pull_request, dict) or not isinstance(files, list):
                raise ValueError("GitHub API responses have unexpected JSON shapes")
            candidate = validate_pull_request(
                pull_request,
                expected_number=args.expected_pr_number,
                repository=args.repository,
                app_login=args.app_login,
            )
            validate_changed_files(files)
            validate_repository(
                candidate,
                workflow_sha=args.workflow_sha,
                repository=args.repository,
                cwd=pathlib.Path.cwd(),
            )
            args.candidate_json.write_text(
                json.dumps(
                    {
                        "merge_sha": candidate.merge_sha,
                        "version": candidate.version,
                    },
                    sort_keys=True,
                )
                + "\n",
                encoding="utf-8",
            )
            print(f"validated {candidate.tag} at {candidate.merge_sha}")
            return 0

        candidate_data = json.loads(args.candidate_json.read_text(encoding="utf-8"))
        candidate = candidate_from_json(candidate_data)
        status = create_or_verify_tag(
            candidate,
            signing_key=args.signing_key,
            trusted_signers=args.trusted_signers,
            remote=args.remote,
            cwd=pathlib.Path.cwd(),
        )
        print(f"{status} {candidate.tag} at {candidate.merge_sha}")
        return 0
    except (
        TypeError,
        ValueError,
        OSError,
        json.JSONDecodeError,
        tomllib.TOMLDecodeError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 2


def candidate_from_json(value: Any) -> Candidate:
    if not isinstance(value, dict) or set(value) != {"merge_sha", "version"}:
        raise ValueError("release candidate JSON has an unexpected shape")
    version = value.get("version")
    merge_sha = value.get("merge_sha")
    if (
        not isinstance(version, str)
        or _version(version) is None
        or not isinstance(merge_sha, str)
        or SHA.fullmatch(merge_sha) is None
    ):
        raise ValueError("release candidate JSON is invalid")
    return Candidate(version=version, merge_sha=merge_sha)


def _git(cwd: pathlib.Path, *args: str) -> str:
    environment = {
        **os.environ,
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_NOSYSTEM": "1",
    }
    result = subprocess.run(
        ["git", *args],
        cwd=cwd,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise ValueError(result.stderr.strip() or "git command failed")
    return result.stdout.strip()


def _version(value: str) -> tuple[int, int, int] | None:
    match = release_policy.VERSION_PATTERN.fullmatch(value)
    return tuple(map(int, match.groups())) if match else None


def validate_repository(
    candidate: Candidate,
    *,
    workflow_sha: str,
    repository: str,
    cwd: pathlib.Path,
) -> None:
    if SHA.fullmatch(workflow_sha) is None:
        raise ValueError("workflow SHA must be a full lowercase commit SHA")
    ancestry = subprocess.run(
        ["git", "merge-base", "--is-ancestor", candidate.merge_sha, workflow_sha],
        cwd=cwd,
        env={**os.environ, "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"},
        check=False,
        capture_output=True,
        text=True,
    )
    if ancestry.returncode != 0:
        raise ValueError("release merge SHA is not an ancestor of the workflow SHA")

    manifest = tomllib.loads(_git(cwd, "show", f"{candidate.merge_sha}:Cargo.toml"))
    lockfile = tomllib.loads(_git(cwd, "show", f"{candidate.merge_sha}:Cargo.lock"))
    changelog = _git(cwd, "show", f"{candidate.merge_sha}:CHANGELOG.md")
    lock_versions = {
        package.get("version")
        for package in lockfile.get("package", [])
        if package.get("name") == "tapas"
    }

    stable_tags = [
        (parsed, tag)
        for tag in _git(cwd, "tag", "--list", "v*").splitlines()
        if (parsed := _version(tag)) is not None
    ]
    if not stable_tags:
        raise ValueError("repository has no stable release tag")
    candidate_version = _version(candidate.tag)
    if candidate_version is None:
        raise ValueError("release candidate version is invalid")
    previous_tags = [item for item in stable_tags if item[0] < candidate_version]
    if not previous_tags:
        raise ValueError("repository has no stable release tag before the candidate")
    candidate_exists = any(tag == candidate.tag for _, tag in stable_tags)
    if not candidate_exists and any(
        parsed >= candidate_version for parsed, _ in stable_tags
    ):
        raise ValueError("release candidate is not newer than every stable tag")
    _, previous_tag = max(previous_tags)
    legal_versions = {
        release_policy.next_version(previous_tag, bump)
        for bump in release_policy.RELEASE_RANK
    }
    comparison = (
        f"## [{candidate.version}](https://github.com/{repository}/compare/"
        f"{previous_tag}...{candidate.tag})"
    )
    if (
        manifest.get("package", {}).get("name") != "tapas"
        or manifest.get("package", {}).get("version") != candidate.version
        or lock_versions != {candidate.version}
        or candidate.version not in legal_versions
        or not any(line.startswith(comparison) for line in changelog.splitlines())
    ):
        raise ValueError("release files or version progression are inconsistent")


def validate_pull_request(
    pull_request: dict[str, Any],
    *,
    expected_number: int,
    repository: str,
    app_login: str,
) -> Candidate:
    nested_objects = (
        pull_request.get("base"),
        pull_request.get("head"),
        pull_request.get("user"),
        pull_request.get("merged_by"),
    )
    if not all(isinstance(value, dict) for value in nested_objects):
        raise ValueError("pull request does not satisfy the trusted release policy")
    if not isinstance(pull_request["head"].get("repo"), dict):
        raise ValueError("pull request does not satisfy the trusted release policy")
    title_match = RELEASE_TITLE.fullmatch(pull_request.get("title", ""))
    merge_sha = pull_request.get("merge_commit_sha", "")
    merger = pull_request.get("merged_by", {})
    trusted_merger = merger.get("type") == "User" or (
        merger.get("type") == "Bot" and merger.get("login") == app_login
    )
    if (
        expected_number < 1
        or pull_request.get("number") != expected_number
        or pull_request.get("state") != "closed"
        or pull_request.get("merged") is not True
        or not pull_request.get("merged_at")
        or pull_request.get("base", {}).get("ref") != "main"
        or pull_request.get("head", {}).get("repo", {}).get("full_name")
        != repository
        or RELEASE_BRANCH.fullmatch(pull_request.get("head", {}).get("ref", ""))
        is None
        or pull_request.get("user", {}).get("login") != app_login
        or pull_request.get("user", {}).get("type") != "Bot"
        or not trusted_merger
        or title_match is None
        or SHA.fullmatch(merge_sha) is None
    ):
        raise ValueError("pull request does not satisfy the trusted release policy")

    version = ".".join(title_match.groups())
    return Candidate(version=version, merge_sha=merge_sha)


if __name__ == "__main__":
    raise SystemExit(main())
