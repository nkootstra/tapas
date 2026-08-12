from __future__ import annotations

import pathlib
import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))

import release_tag  # noqa: E402


def git(repo: pathlib.Path, *args: str, check: bool = True) -> str:
    environment = {
        **os.environ,
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_NOSYSTEM": "1",
    }
    result = subprocess.run(
        ["git", *args],
        cwd=repo,
        env=environment,
        check=check,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def commit(repo: pathlib.Path, message: str) -> str:
    git(repo, "add", ".")
    git(
        repo,
        "-c",
        "user.name=Test User",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-m",
        message,
    )
    return git(repo, "rev-parse", "HEAD")


class CandidateValidationTests(unittest.TestCase):
    def test_accepts_trusted_merged_release_pull_request(self) -> None:
        pull_request = {
            "number": 15,
            "state": "closed",
            "merged": True,
            "merged_at": "2026-08-11T10:00:00Z",
            "title": "skip: prepare v0.4.0",
            "base": {"ref": "main"},
            "head": {
                "ref": "release-plz-2026-08-11T09-00-00Z",
                "repo": {"full_name": "nkootstra/tapas"},
            },
            "user": {"login": "tapas-release[bot]", "type": "Bot"},
            "merged_by": {"login": "nkootstra", "type": "User"},
            "merge_commit_sha": "e06fa6388fe9ae24a51db09314c873ddbe6816bf",
        }

        candidate = release_tag.validate_pull_request(
            pull_request,
            expected_number=15,
            repository="nkootstra/tapas",
            app_login="tapas-release[bot]",
        )

        self.assertEqual(candidate.version, "0.4.0")
        self.assertEqual(candidate.tag, "v0.4.0")
        self.assertEqual(
            candidate.merge_sha, "e06fa6388fe9ae24a51db09314c873ddbe6816bf"
        )

    def test_rejects_untrusted_pull_request_metadata(self) -> None:
        valid = {
            "number": 15,
            "state": "closed",
            "merged": True,
            "merged_at": "2026-08-11T10:00:00Z",
            "title": "skip: prepare v0.4.0",
            "base": {"ref": "main"},
            "head": {
                "ref": "release-plz-2026-08-11T09-00-00Z",
                "repo": {"full_name": "nkootstra/tapas"},
            },
            "user": {"login": "tapas-release[bot]", "type": "Bot"},
            "merged_by": {"login": "nkootstra", "type": "User"},
            "merge_commit_sha": "e06fa6388fe9ae24a51db09314c873ddbe6816bf",
        }
        mutations = {
            "number": ("number", 14),
            "open": ("state", "open"),
            "not merged": ("merged", False),
            "unmerged": ("merged_at", None),
            "title": ("title", "skip: prepare v00.4.0"),
            "sha": ("merge_commit_sha", "not-a-sha"),
        }
        nested_mutations = {
            "base": ("base", "ref", "develop"),
            "fork": ("head", "repo", {"full_name": "attacker/tapas"}),
            "branch": ("head", "ref", "release-plz-"),
            "author": ("user", "login", "attacker[bot]"),
            "author type": ("user", "type", "User"),
            "merger": ("merged_by", "type", "Bot"),
        }

        for name, (key, value) in mutations.items():
            with self.subTest(name=name):
                pull_request = copy.deepcopy(valid)
                pull_request[key] = value
                with self.assertRaises(ValueError):
                    release_tag.validate_pull_request(
                        pull_request,
                        expected_number=15,
                        repository="nkootstra/tapas",
                        app_login="tapas-release[bot]",
                    )

        for name, (outer, inner, value) in nested_mutations.items():
            with self.subTest(name=name):
                pull_request = copy.deepcopy(valid)
                pull_request[outer][inner] = value
                with self.assertRaises(ValueError):
                    release_tag.validate_pull_request(
                        pull_request,
                        expected_number=15,
                        repository="nkootstra/tapas",
                        app_login="tapas-release[bot]",
                    )

        malformed = copy.deepcopy(valid)
        malformed["head"] = []
        with self.assertRaises(ValueError):
            release_tag.validate_pull_request(
                malformed,
                expected_number=15,
                repository="nkootstra/tapas",
                app_login="tapas-release[bot]",
            )

    def test_requires_exact_release_file_set(self) -> None:
        expected = [
            {"filename": "Cargo.toml"},
            {"filename": "Cargo.lock"},
            {"filename": "CHANGELOG.md"},
        ]

        self.assertIsNone(release_tag.validate_changed_files(expected))
        for files in (
            expected[:-1],
            [*expected, {"filename": "src/main.rs"}],
            [{"filename": "cargo.toml"}, *expected[1:]],
        ):
            with self.subTest(files=files), self.assertRaises(ValueError):
                release_tag.validate_changed_files(files)

    def test_cli_fails_closed_before_git_for_invalid_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            pr_json = root / "pr.json"
            files_json = root / "files.json"
            pr_json.write_text(json.dumps({"number": 15, "state": "open"}))
            files_json.write_text(json.dumps([]))

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPTS / "release_tag.py"),
                    "validate",
                    "--pr-json",
                    str(pr_json),
                    "--files-json",
                    str(files_json),
                    "--expected-pr-number",
                    "15",
                    "--repository",
                    "nkootstra/tapas",
                    "--app-login",
                    "tapas-release[bot]",
                    "--workflow-sha",
                    "a" * 40,
                    "--candidate-json",
                    str(root / "candidate.json"),
                ],
                check=False,
                capture_output=True,
                text=True,
                env={
                    **os.environ,
                    "GIT_CONFIG_GLOBAL": "/dev/null",
                    "GIT_CONFIG_NOSYSTEM": "1",
                },
            )

            self.assertEqual(result.returncode, 2)
            self.assertIn("trusted release policy", result.stderr)

    def test_cli_fails_closed_for_non_string_json_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            pr_json = root / "pr.json"
            files_json = root / "files.json"
            pr_json.write_text(
                json.dumps(
                    {
                        "number": 15,
                        "state": "closed",
                        "merged": True,
                        "merged_at": "2026-08-11T10:00:00Z",
                        "title": 123,
                        "base": {"ref": "main"},
                        "head": {
                            "ref": "release-plz-2026-08-11T09-00-00Z",
                            "repo": {"full_name": "nkootstra/tapas"},
                        },
                        "user": {"login": "tapas-release[bot]", "type": "Bot"},
                        "merged_by": {"login": "nkootstra", "type": "User"},
                        "merge_commit_sha": "a" * 40,
                    }
                )
            )
            files_json.write_text(
                json.dumps(
                    [
                        {"filename": "Cargo.toml"},
                        {"filename": "Cargo.lock"},
                        {"filename": "CHANGELOG.md"},
                    ]
                )
            )

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPTS / "release_tag.py"),
                    "validate",
                    "--pr-json",
                    str(pr_json),
                    "--files-json",
                    str(files_json),
                    "--expected-pr-number",
                    "15",
                    "--repository",
                    "nkootstra/tapas",
                    "--app-login",
                    "tapas-release[bot]",
                    "--workflow-sha",
                    "a" * 40,
                    "--candidate-json",
                    str(root / "candidate.json"),
                ],
                check=False,
                capture_output=True,
                text=True,
                env={
                    **os.environ,
                    "GIT_CONFIG_GLOBAL": "/dev/null",
                    "GIT_CONFIG_NOSYSTEM": "1",
                },
            )

            self.assertEqual(result.returncode, 2)
            self.assertNotIn("Traceback", result.stderr)


class RepositoryValidationTests(unittest.TestCase):
    def test_accepts_consistent_next_version_at_ancestor_sha(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = pathlib.Path(directory)
            git(repo, "init", "-b", "main")
            (repo / "Cargo.toml").write_text(
                '[package]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "CHANGELOG.md").write_text("# Changelog\n")
            commit(repo, "initial")
            git(repo, "tag", "v0.3.0")

            (repo / "Cargo.toml").write_text(
                '[package]\nname = "tapas"\nversion = "0.4.0"\n'
            )
            (repo / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "tapas"\nversion = "0.4.0"\n'
            )
            (repo / "CHANGELOG.md").write_text(
                "# Changelog\n\n"
                "## [0.4.0](https://github.com/nkootstra/tapas/compare/"
                "v0.3.0...v0.4.0) - 2026-08-11\n"
            )
            merge_sha = commit(repo, "release")
            (repo / "README.md").write_text("trusted workflow\n")
            workflow_sha = commit(repo, "workflow")

            release_tag.validate_repository(
                release_tag.Candidate("0.4.0", merge_sha),
                workflow_sha=workflow_sha,
                repository="nkootstra/tapas",
                cwd=repo,
            )

    def test_rejects_merge_sha_outside_trusted_workflow_history(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = pathlib.Path(directory)
            git(repo, "init", "-b", "main")
            (repo / "Cargo.toml").write_text(
                '[package]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "CHANGELOG.md").write_text("# Changelog\n")
            workflow_sha = commit(repo, "trusted workflow")
            git(repo, "tag", "v0.3.0")
            git(repo, "switch", "--orphan", "untrusted")
            (repo / "Cargo.toml").write_text(
                '[package]\nname = "tapas"\nversion = "0.4.0"\n'
            )
            (repo / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "tapas"\nversion = "0.4.0"\n'
            )
            (repo / "CHANGELOG.md").write_text(
                "# Changelog\n\n"
                "## [0.4.0](https://github.com/nkootstra/tapas/compare/"
                "v0.3.0...v0.4.0) - 2026-08-11\n"
            )
            merge_sha = commit(repo, "untrusted release")

            with self.assertRaisesRegex(ValueError, "not an ancestor"):
                release_tag.validate_repository(
                    release_tag.Candidate("0.4.0", merge_sha),
                    workflow_sha=workflow_sha,
                    repository="nkootstra/tapas",
                    cwd=repo,
                )

    def test_rejects_invalid_candidate_version_with_value_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = pathlib.Path(directory)
            git(repo, "init", "-b", "main")
            (repo / "Cargo.toml").write_text(
                '[package]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "CHANGELOG.md").write_text("# Changelog\n")
            workflow_sha = commit(repo, "trusted workflow")
            git(repo, "tag", "v0.3.0")

            with self.assertRaisesRegex(ValueError, "candidate version is invalid"):
                release_tag.validate_repository(
                    release_tag.Candidate("invalid", workflow_sha),
                    workflow_sha=workflow_sha,
                    repository="nkootstra/tapas",
                    cwd=repo,
                )


class SigningKeyValidationTests(unittest.TestCase):
    def test_requires_private_key_to_match_a_trusted_signer(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            signing_key = root / "release-key"
            other_key = root / "other-key"
            subprocess.run(
                [
                    "/usr/bin/ssh-keygen",
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-f",
                    str(signing_key),
                ],
                check=True,
            )
            subprocess.run(
                [
                    "/usr/bin/ssh-keygen",
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-f",
                    str(other_key),
                ],
                check=True,
            )
            trusted = root / "allowed-signers"
            trusted.write_text(
                "release@example.com namespaces=\"git\" "
                f"{signing_key.with_suffix('.pub').read_text()}"
            )

            public_key, identity = release_tag.validate_signing_key(
                signing_key, trusted
            )
            self.assertTrue(public_key.startswith("ssh-ed25519 "))
            self.assertEqual(identity, "release@example.com")

            trusted.write_text(
                "other@example.com namespaces=\"git\" "
                f"{other_key.with_suffix('.pub').read_text()}"
            )
            with self.assertRaises(ValueError):
                release_tag.validate_signing_key(signing_key, trusted)

    def test_ignores_blank_and_commented_allowed_signer_lines(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            signing_key = root / "release-key"
            other_key = root / "other-key"
            for key in (signing_key, other_key):
                subprocess.run(
                    [
                        "/usr/bin/ssh-keygen",
                        "-q",
                        "-t",
                        "ed25519",
                        "-N",
                        "",
                        "-f",
                        str(key),
                    ],
                    check=True,
                )
            trusted = root / "allowed-signers"
            trusted.write_text(
                "\n"
                "# retired@example.com namespaces=\"git\" "
                f"{signing_key.with_suffix('.pub').read_text()}"
                "active@example.com namespaces=\"git\" "
                f"{other_key.with_suffix('.pub').read_text()}"
            )

            with self.assertRaises(ValueError):
                release_tag.validate_signing_key(signing_key, trusted)


class SignedTagIntegrationTests(unittest.TestCase):
    def test_verifies_and_pushes_valid_local_tag_when_remote_tag_is_absent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            remote = root / "remote.git"
            repo = root / "work"
            git(root, "init", "--bare", str(remote))
            git(root, "init", "-b", "main", str(repo))
            (repo / "release.txt").write_text("candidate\n")
            merge_sha = commit(repo, "release candidate")
            git(repo, "remote", "add", "origin", str(remote))
            git(repo, "push", "origin", "main")

            signing_key = root / "release-key"
            subprocess.run(
                [
                    "/usr/bin/ssh-keygen",
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-f",
                    str(signing_key),
                ],
                check=True,
            )
            trusted = root / "allowed-signers"
            trusted.write_text(
                "release@example.com namespaces=\"git\" "
                f"{signing_key.with_suffix('.pub').read_text()}"
            )
            git(
                repo,
                "-c",
                "user.name=Tapas Release",
                "-c",
                "user.email=release@example.com",
                "-c",
                "gpg.format=ssh",
                "-c",
                "gpg.ssh.program=/usr/bin/ssh-keygen",
                "-c",
                f"user.signingkey={signing_key}",
                "tag",
                "--sign",
                "--annotate",
                "v0.4.0",
                merge_sha,
                "--message",
                "Release v0.4.0",
            )
            self.assertEqual(
                git(repo, "ls-remote", "--tags", "origin", "refs/tags/v0.4.0"),
                "",
            )

            status = release_tag.create_or_verify_tag(
                release_tag.Candidate("0.4.0", merge_sha),
                signing_key=signing_key,
                trusted_signers=trusted,
                remote="origin",
                cwd=repo,
            )

            self.assertEqual(status, "created")
            remote_tag = git(
                repo, "ls-remote", "--tags", "origin", "refs/tags/v0.4.0"
            )
            self.assertTrue(remote_tag.endswith("refs/tags/v0.4.0"))

    def test_signs_exact_historical_sha_pushes_replays_and_rejects_conflict(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            remote = root / "remote.git"
            repo = root / "work"
            git(root, "init", "--bare", str(remote))
            git(root, "init", "-b", "main", str(repo))
            (repo / "release.txt").write_text("candidate\n")
            merge_sha = commit(repo, "release candidate")
            (repo / "workflow.yml").write_text("trusted workflow\n")
            workflow_sha = commit(repo, "trusted workflow")
            git(repo, "remote", "add", "origin", str(remote))
            git(repo, "push", "origin", "main")
            git(repo, "switch", "--detach", workflow_sha)

            signing_key = root / "release-key"
            subprocess.run(
                [
                    "/usr/bin/ssh-keygen",
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-f",
                    str(signing_key),
                ],
                check=True,
            )
            trusted = root / "allowed-signers"
            trusted.write_text(
                "release@example.com namespaces=\"git\" "
                f"{signing_key.with_suffix('.pub').read_text()}"
            )
            candidate = release_tag.Candidate("0.4.0", merge_sha)

            self.assertEqual(
                release_tag.create_or_verify_tag(
                    candidate,
                    signing_key=signing_key,
                    trusted_signers=trusted,
                    remote="origin",
                    cwd=repo,
                ),
                "created",
            )
            self.assertEqual(git(repo, "rev-list", "-n", "1", "v0.4.0"), merge_sha)
            self.assertEqual(git(repo, "cat-file", "-t", "v0.4.0"), "tag")
            self.assertEqual(
                release_tag.create_or_verify_tag(
                    candidate,
                    signing_key=signing_key,
                    trusted_signers=trusted,
                    remote="origin",
                    cwd=repo,
                ),
                "existing",
            )

            git(repo, "tag", "-f", "v0.4.0", workflow_sha)
            git(repo, "push", "--force", "origin", "refs/tags/v0.4.0")
            with self.assertRaises(ValueError):
                release_tag.create_or_verify_tag(
                    candidate,
                    signing_key=signing_key,
                    trusted_signers=trusted,
                    remote="origin",
                    cwd=repo,
                )

    def test_rejects_exact_target_tag_signed_by_an_untrusted_key(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            remote = root / "remote.git"
            repo = root / "work"
            git(root, "init", "--bare", str(remote))
            git(root, "init", "-b", "main", str(repo))
            (repo / "release.txt").write_text("candidate\n")
            merge_sha = commit(repo, "release candidate")
            git(repo, "remote", "add", "origin", str(remote))
            git(repo, "push", "origin", "main")

            trusted_key = root / "trusted-key"
            untrusted_key = root / "untrusted-key"
            for key in (trusted_key, untrusted_key):
                subprocess.run(
                    [
                        "/usr/bin/ssh-keygen",
                        "-q",
                        "-t",
                        "ed25519",
                        "-N",
                        "",
                        "-f",
                        str(key),
                    ],
                    check=True,
                )
            trusted = root / "allowed-signers"
            trusted.write_text(
                "release@example.com namespaces=\"git\" "
                f"{trusted_key.with_suffix('.pub').read_text()}"
            )
            git(
                repo,
                "-c",
                "user.name=Untrusted",
                "-c",
                "user.email=untrusted@example.com",
                "-c",
                "gpg.format=ssh",
                "-c",
                "gpg.ssh.program=/usr/bin/ssh-keygen",
                "-c",
                f"user.signingkey={untrusted_key}",
                "tag",
                "--sign",
                "--annotate",
                "v0.4.0",
                merge_sha,
                "--message",
                "Release v0.4.0",
            )
            git(repo, "push", "origin", "refs/tags/v0.4.0")

            with self.assertRaises(ValueError):
                release_tag.create_or_verify_tag(
                    release_tag.Candidate("0.4.0", merge_sha),
                    signing_key=trusted_key,
                    trusted_signers=trusted,
                    remote="origin",
                    cwd=repo,
                )

    def test_replays_valid_older_tag_after_a_newer_release(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            remote = root / "remote.git"
            repo = root / "work"
            git(root, "init", "--bare", str(remote))
            git(root, "init", "-b", "main", str(repo))
            (repo / "Cargo.toml").write_text(
                '[package]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "CHANGELOG.md").write_text("# Changelog\n")
            commit(repo, "v0.3.0")
            git(repo, "tag", "v0.3.0")
            (repo / "Cargo.toml").write_text(
                '[package]\nname = "tapas"\nversion = "0.4.0"\n'
            )
            (repo / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "tapas"\nversion = "0.4.0"\n'
            )
            (repo / "CHANGELOG.md").write_text(
                "# Changelog\n\n"
                "## [0.4.0](https://github.com/nkootstra/tapas/compare/"
                "v0.3.0...v0.4.0) - 2026-08-11\n"
            )
            merge_sha = commit(repo, "v0.4.0")
            (repo / "workflow.yml").write_text("trusted workflow\n")
            workflow_sha = commit(repo, "workflow")
            git(repo, "tag", "v0.5.0")
            git(repo, "remote", "add", "origin", str(remote))
            git(repo, "push", "origin", "main", "refs/tags/v0.3.0", "refs/tags/v0.5.0")

            signing_key = root / "release-key"
            subprocess.run(
                [
                    "/usr/bin/ssh-keygen",
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-f",
                    str(signing_key),
                ],
                check=True,
            )
            trusted = root / "allowed-signers"
            trusted.write_text(
                "release@example.com namespaces=\"git\" "
                f"{signing_key.with_suffix('.pub').read_text()}"
            )
            candidate = release_tag.Candidate("0.4.0", merge_sha)
            self.assertEqual(
                release_tag.create_or_verify_tag(
                    candidate,
                    signing_key=signing_key,
                    trusted_signers=trusted,
                    remote="origin",
                    cwd=repo,
                ),
                "created",
            )

            release_tag.validate_repository(
                candidate,
                workflow_sha=workflow_sha,
                repository="nkootstra/tapas",
                cwd=repo,
            )
            self.assertEqual(
                release_tag.create_or_verify_tag(
                    candidate,
                    signing_key=signing_key,
                    trusted_signers=trusted,
                    remote="origin",
                    cwd=repo,
                ),
                "existing",
            )

    def test_cli_creates_release_from_github_api_responses(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            remote = root / "remote.git"
            repo = root / "work"
            git(root, "init", "--bare", str(remote))
            git(root, "init", "-b", "main", str(repo))
            (repo / "Cargo.toml").write_text(
                '[package]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "tapas"\nversion = "0.3.0"\n'
            )
            (repo / "CHANGELOG.md").write_text("# Changelog\n")
            commit(repo, "initial")
            git(repo, "tag", "v0.3.0")
            (repo / "Cargo.toml").write_text(
                '[package]\nname = "tapas"\nversion = "0.4.0"\n'
            )
            (repo / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "tapas"\nversion = "0.4.0"\n'
            )
            (repo / "CHANGELOG.md").write_text(
                "# Changelog\n\n"
                "## [0.4.0](https://github.com/nkootstra/tapas/compare/"
                "v0.3.0...v0.4.0) - 2026-08-11\n"
            )
            merge_sha = commit(repo, "release")
            (repo / "workflow.yml").write_text("trusted workflow\n")
            workflow_sha = commit(repo, "workflow")
            git(repo, "remote", "add", "origin", str(remote))
            git(repo, "push", "origin", "main", "refs/tags/v0.3.0")

            signing_key = root / "release-key"
            subprocess.run(
                [
                    "/usr/bin/ssh-keygen",
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-f",
                    str(signing_key),
                ],
                check=True,
            )
            trusted = root / "allowed-signers"
            trusted.write_text(
                "release@example.com namespaces=\"git\" "
                f"{signing_key.with_suffix('.pub').read_text()}"
            )
            pr_json = root / "pr.json"
            files_json = root / "files.json"
            candidate_json = root / "candidate.json"
            pr_json.write_text(
                json.dumps(
                    {
                        "number": 15,
                        "state": "closed",
                        "merged": True,
                        "merged_at": "2026-08-11T10:00:00Z",
                        "title": "skip: prepare v0.4.0",
                        "base": {"ref": "main"},
                        "head": {
                            "ref": "release-plz-2026-08-11T09-00-00Z",
                            "repo": {"full_name": "nkootstra/tapas"},
                        },
                        "user": {
                            "login": "tapas-release[bot]",
                            "type": "Bot",
                            "id": 123,
                        },
                        "merged_by": {"login": "nkootstra", "type": "User"},
                        "merge_commit_sha": merge_sha,
                    }
                )
            )
            files_json.write_text(
                json.dumps(
                    [
                        {"filename": "Cargo.toml", "status": "modified"},
                        {"filename": "Cargo.lock", "status": "modified"},
                        {"filename": "CHANGELOG.md", "status": "modified"},
                    ]
                )
            )

            validation = subprocess.run(
                [
                    "python3",
                    str(SCRIPTS / "release_tag.py"),
                    "validate",
                    "--pr-json",
                    str(pr_json),
                    "--files-json",
                    str(files_json),
                    "--expected-pr-number",
                    "15",
                    "--repository",
                    "nkootstra/tapas",
                    "--app-login",
                    "tapas-release[bot]",
                    "--workflow-sha",
                    workflow_sha,
                    "--candidate-json",
                    str(candidate_json),
                ],
                cwd=repo,
                check=False,
                capture_output=True,
                text=True,
                env={
                    **os.environ,
                    "GIT_CONFIG_GLOBAL": "/dev/null",
                    "GIT_CONFIG_NOSYSTEM": "1",
                },
            )
            self.assertEqual(validation.returncode, 0, validation.stderr)
            self.assertEqual(
                json.loads(candidate_json.read_text()),
                {"merge_sha": merge_sha, "version": "0.4.0"},
            )

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPTS / "release_tag.py"),
                    "tag",
                    "--candidate-json",
                    str(candidate_json),
                    "--signing-key",
                    str(signing_key),
                    "--trusted-signers",
                    str(trusted),
                    "--remote",
                    "origin",
                ],
                cwd=repo,
                check=False,
                capture_output=True,
                text=True,
                env={
                    **os.environ,
                    "GIT_CONFIG_GLOBAL": "/dev/null",
                    "GIT_CONFIG_NOSYSTEM": "1",
                },
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"created v0.4.0 at {merge_sha}", result.stdout)

            replay = subprocess.run(
                result.args,
                cwd=repo,
                check=False,
                capture_output=True,
                text=True,
                env={
                    **os.environ,
                    "GIT_CONFIG_GLOBAL": "/dev/null",
                    "GIT_CONFIG_NOSYSTEM": "1",
                },
            )
            self.assertEqual(replay.returncode, 0, replay.stderr)
            self.assertIn(f"existing v0.4.0 at {merge_sha}", replay.stdout)

if __name__ == "__main__":
    unittest.main()
