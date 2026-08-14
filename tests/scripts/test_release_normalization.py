from __future__ import annotations

import base64
import json
import pathlib
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import release_normalization  # noqa: E402


class ReleaseNormalizationTests(unittest.TestCase):
    def test_selects_exactly_one_trusted_open_release_pull_request(self) -> None:
        trusted = {
            "number": 21,
            "title": "skip: prepare v0.4.0",
            "user": {"login": "tapas-release[bot]", "type": "Bot"},
            "head": {
                "ref": "release-plz-2026-08-11",
                "sha": "a" * 40,
                "repo": {"full_name": "nkootstra/tapas"},
            },
        }
        unrelated = {
            **trusted,
            "number": 22,
            "user": {"login": "someone", "type": "User"},
        }

        selected = release_normalization.select_release_pull_request(
            [unrelated, trusted],
            repository="nkootstra/tapas",
            app_login="tapas-release[bot]",
        )

        self.assertEqual(selected, trusted)
        for candidates in ([], [trusted, dict(trusted)]):
            with self.subTest(candidates=candidates), self.assertRaises(ValueError):
                release_normalization.select_release_pull_request(
                    candidates,
                    repository="nkootstra/tapas",
                    app_login="tapas-release[bot]",
                )

    def test_inspect_rejects_non_string_release_branch_with_status_2(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            pulls_json = root / "pulls.json"
            output_json = root / "selected.json"
            pulls_json.write_text(
                json.dumps(
                    [
                        {
                            "user": {"login": "tapas-release[bot]", "type": "Bot"},
                            "head": {
                                "ref": 123,
                                "repo": {"full_name": "nkootstra/tapas"},
                            },
                        }
                    ]
                ),
                encoding="utf-8",
            )

            status = release_normalization.main(
                [
                    "inspect",
                    "--pulls-json",
                    str(pulls_json),
                    "--repository",
                    "nkootstra/tapas",
                    "--app-login",
                    "tapas-release[bot]",
                    "--output-json",
                    str(output_json),
                ]
            )

            self.assertEqual(status, 2)
            self.assertFalse(output_json.exists())

    def test_inspect_allows_no_existing_release_pull_request(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            pulls_json = root / "pulls.json"
            output_json = root / "selected.json"
            pulls_json.write_text("[]\n", encoding="utf-8")

            status = release_normalization.main(
                [
                    "inspect",
                    "--pulls-json",
                    str(pulls_json),
                    "--repository",
                    "nkootstra/tapas",
                    "--app-login",
                    "tapas-release[bot]",
                    "--output-json",
                    str(output_json),
                    "--allow-none",
                ]
            )

            self.assertEqual(status, 0)
            self.assertEqual(json.loads(output_json.read_text()), {})

    def test_validates_release_pull_request_before_auto_merge(self) -> None:
        pull_request = self._trusted_open_pull_request()
        files = self._release_files()

        candidate = release_normalization.validate_auto_merge_candidate(
            pull_request,
            files,
            repository="nkootstra/tapas",
            app_login="tapas-release[bot]",
        )

        self.assertEqual(candidate.number, 21)
        self.assertEqual(candidate.head_sha, "a" * 40)
        self.assertEqual(candidate.title, "skip: prepare v0.4.0")

    def test_validates_existing_app_owned_squash_auto_merge(self) -> None:
        pull_request = {
            **self._trusted_open_pull_request(),
            "auto_merge": {
                "merge_method": "squash",
                "enabled_by": {
                    "login": "tapas-release[bot]",
                    "type": "Bot",
                },
            },
        }

        candidate = release_normalization.validate_auto_merge_candidate(
            pull_request,
            self._release_files(),
            repository="nkootstra/tapas",
            app_login="tapas-release[bot]",
        )

        self.assertTrue(candidate.disable_existing)

    def test_auto_merge_command_serializes_validated_candidate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            pr_json = root / "pr.json"
            files_json = root / "files.json"
            output_json = root / "candidate.json"
            pr_json.write_text(json.dumps(self._trusted_open_pull_request()))
            files_json.write_text(json.dumps(self._release_files()))

            status = release_normalization.main(
                [
                    "auto-merge",
                    "--pr-json",
                    str(pr_json),
                    "--files-json",
                    str(files_json),
                    "--repository",
                    "nkootstra/tapas",
                    "--app-login",
                    "tapas-release[bot]",
                    "--output-json",
                    str(output_json),
                ]
            )

            self.assertEqual(status, 0)
            self.assertEqual(
                json.loads(output_json.read_text()),
                {
                    "disable_existing": False,
                    "head_sha": "a" * 40,
                    "number": 21,
                    "title": "skip: prepare v0.4.0",
                },
            )

    def test_rejects_drifted_release_pull_request_before_auto_merge(self) -> None:
        trusted = self._trusted_open_pull_request()
        files = self._release_files()
        mutations = (
            {**trusted, "draft": True},
            {**trusted, "title": "skip: prepare v0.4"},
            {**trusted, "state": "closed"},
            {**trusted, "auto_merge": {"merge_method": "merge"}},
            {**trusted, "head": {**trusted["head"], "ref": ["not", "text"]}},
        )

        for pull_request in mutations:
            with self.subTest(pull_request=pull_request), self.assertRaises(ValueError):
                release_normalization.validate_auto_merge_candidate(
                    pull_request,
                    files,
                    repository="nkootstra/tapas",
                    app_login="tapas-release[bot]",
                )
        with self.assertRaises(ValueError):
            release_normalization.validate_auto_merge_candidate(
                trusted,
                [*files, {"filename": "src/main.rs"}],
                repository="nkootstra/tapas",
                app_login="tapas-release[bot]",
            )
        with self.assertRaises(ValueError):
            release_normalization.validate_auto_merge_candidate(
                trusted,
                [*files[:2], {"filename": ["not", "text"]}],
                repository="nkootstra/tapas",
                app_login="tapas-release[bot]",
            )

    @staticmethod
    def _trusted_open_pull_request() -> dict[str, object]:
        return {
            "number": 21,
            "title": "skip: prepare v0.4.0",
            "state": "open",
            "draft": False,
            "base": {"ref": "main"},
            "user": {"login": "tapas-release[bot]", "type": "Bot"},
            "head": {
                "ref": "release-plz-2026-08-11",
                "sha": "a" * 40,
                "repo": {"full_name": "nkootstra/tapas"},
            },
            "auto_merge": None,
        }

    @staticmethod
    def _release_files() -> list[dict[str, str]]:
        return [
            {"filename": "Cargo.toml"},
            {"filename": "Cargo.lock"},
            {"filename": "CHANGELOG.md"},
        ]

    def test_builds_expected_head_graphql_commit_from_exact_release_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            manifest = root / "Cargo.toml"
            lockfile = root / "Cargo.lock"
            manifest.write_bytes(b"manifest\n")
            lockfile.write_bytes(b"lock\n")

            request = release_normalization.build_commit_request(
                repository="nkootstra/tapas",
                branch="release-plz-2026-08-11",
                expected_head="a" * 40,
                version="0.4.0",
                files=[manifest, lockfile],
                root=root,
            )

            commit_input = request["variables"]["input"]
            self.assertIn("createCommitOnBranch", request["query"])
            self.assertEqual(commit_input["expectedHeadOid"], "a" * 40)
            self.assertEqual(
                commit_input["branch"],
                {
                    "repositoryNameWithOwner": "nkootstra/tapas",
                    "branchName": "release-plz-2026-08-11",
                },
            )
            self.assertEqual(
                commit_input["message"]["headline"],
                "skip: normalize release version to v0.4.0",
            )
            additions = commit_input["fileChanges"]["additions"]
            self.assertEqual([item["path"] for item in additions], ["Cargo.toml", "Cargo.lock"])
            self.assertEqual(base64.b64decode(additions[0]["contents"]), b"manifest\n")

            unexpected = root / "src.rs"
            unexpected.write_text("untrusted\n")
            with self.assertRaises(ValueError):
                release_normalization.build_commit_request(
                    repository="nkootstra/tapas",
                    branch="release-plz-2026-08-11",
                    expected_head="a" * 40,
                    version="0.4.0",
                    files=[unexpected],
                    root=root,
                )

    def test_normalizes_body_idempotently_and_fails_closed(self) -> None:
        body = "Release 0.3.1\nCompare 0.3.1\n"
        normalized = release_normalization.normalize_body(body, "0.3.1", "0.4.0")
        self.assertEqual(normalized, "Release 0.4.0\nCompare 0.4.0\n")
        self.assertEqual(
            release_normalization.normalize_body(normalized, "0.3.1", "0.4.0"),
            normalized,
        )
        with self.assertRaises(ValueError):
            release_normalization.normalize_body("no version\n", "0.3.1", "0.4.0")

    def test_normalizes_only_standalone_old_version_occurrences(self) -> None:
        body = "Previous 10.3.1\nRelease 0.3.1\n"

        normalized = release_normalization.normalize_body(body, "0.3.1", "0.4.0")

        self.assertEqual(normalized, "Previous 10.3.1\nRelease 0.4.0\n")
        with self.assertRaises(ValueError):
            release_normalization.normalize_body(
                "Previous 10.3.1\n", "0.3.1", "0.4.0"
            )
        self.assertEqual(
            release_normalization.normalize_body(
                "Previous 10.4.0\nRelease 0.3.1\n", "0.3.1", "0.4.0"
            ),
            "Previous 10.4.0\nRelease 0.4.0\n",
        )


if __name__ == "__main__":
    unittest.main()
