from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import tempfile
import textwrap
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))

import release_policy  # noqa: E402


def write_version_files(
    root: pathlib.Path, version: str
) -> tuple[pathlib.Path, pathlib.Path]:
    manifest = root / "Cargo.toml"
    lockfile = root / "Cargo.lock"
    manifest.write_text(
        f'[package]\nname = "tapas"\nversion = "{version}"\n',
        encoding="utf-8",
    )
    lockfile.write_text(
        f'[[package]]\nname = "tapas"\nversion = "{version}"\n',
        encoding="utf-8",
    )
    return manifest, lockfile


class PullRequestTitleTests(unittest.TestCase):
    def test_accepts_release_skip_and_ordinary_titles(self) -> None:
        for title in (
            "major: remove the legacy protocol",
            "minor: add release automation",
            "patch: repair tag validation",
            "skip: update internal documentation",
            "feat: add release automation",
            "Improve contributor documentation",
        ):
            with self.subTest(title=title):
                self.assertEqual(release_policy.validate_title(title), None)

    def test_rejects_titles_outside_the_release_contract(self) -> None:
        for title in (
            "Major: remove the legacy protocol",
            "patch:no separating space",
            "skip: ",
            "minor: first line\nsecond line",
            "   ",
        ):
            with self.subTest(title=title):
                with self.assertRaises(ValueError):
                    release_policy.validate_title(title)

    def test_checkout_free_workflow_validator_matches_release_policy(self) -> None:
        workflow = (ROOT / ".github/workflows/pr-title.yml").read_text(
            encoding="utf-8"
        )
        validator = textwrap.dedent(
            workflow.split("# release-title-validator:start", 1)[1].split(
                "# release-title-validator:end", 1
            )[0]
        )
        titles = (
            "minor: add release automation",
            "Improve contributor documentation",
            "Major: wrong case",
            "patch:no separating space",
            "skip: ",
            "minor: first line\nsecond line",
            "   ",
        )

        for title in titles:
            with self.subTest(title=title):
                try:
                    release_policy.validate_title(title)
                except ValueError:
                    expected = 1
                else:
                    expected = 0
                result = subprocess.run(
                    ["python3", "-"],
                    input=validator,
                    env={**os.environ, "PR_TITLE": title},
                    text=True,
                    capture_output=True,
                    check=False,
                )
                self.assertEqual(result.returncode, expected, result.stderr)


class VersionPolicyTests(unittest.TestCase):
    def test_highest_release_intent_wins(self) -> None:
        subjects = [
            "patch: repair output (#1)",
            "skip: update docs (#2)",
            "minor: add command support (#3)",
            "major: remove an incompatible interface (#4)",
        ]

        self.assertEqual(release_policy.select_bump(subjects), "major")

    def test_skip_only_history_does_not_release(self) -> None:
        self.assertIsNone(
            release_policy.select_bump(
                ["skip: update docs (#1)", "chore: historical commit (#2)"]
            )
        )

    def test_only_exact_valid_release_intent_triggers_a_bump(self) -> None:
        self.assertIsNone(
            release_policy.select_bump(
                [
                    "minor:no separating space",
                    "Minor: wrong case",
                    "patch: ",
                    "major: first line\nsecond line",
                    "ordinary maintenance",
                ]
            )
        )

    def test_literal_semver_bumps_include_pre_one_major(self) -> None:
        self.assertEqual(release_policy.next_version("v0.3.0", "patch"), "0.3.1")
        self.assertEqual(release_policy.next_version("v0.3.0", "minor"), "0.4.0")
        self.assertEqual(release_policy.next_version("v0.3.0", "major"), "1.0.0")
        self.assertEqual(release_policy.next_version("v1.8.4", "major"), "2.0.0")

    def test_invalid_versions_and_bumps_fail_closed(self) -> None:
        with self.assertRaises(ValueError):
            release_policy.next_version("0.3", "patch")
        with self.assertRaises(ValueError):
            release_policy.next_version("v0.3.0", "skip")

    def test_release_is_pending_only_until_repository_contains_desired_version(
        self,
    ) -> None:
        subjects = ["minor: add support"]

        self.assertEqual(
            release_policy.pending_release_version("v0.3.0", "0.3.0", subjects),
            "0.4.0",
        )
        self.assertIsNone(
            release_policy.pending_release_version("v0.3.0", "0.4.0", subjects)
        )
        with self.assertRaises(ValueError):
            release_policy.pending_release_version("v0.3.0", "0.3.1", subjects)

    def test_skip_only_history_rejects_unexpected_repository_version(self) -> None:
        with self.assertRaises(ValueError):
            release_policy.pending_release_version(
                "v0.3.0", "0.3.1", ["skip: update docs"]
            )

    def test_repository_version_requires_matching_manifest_and_lockfile(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            manifest, lockfile = write_version_files(root, "0.4.0")

            self.assertEqual(
                release_policy.repository_version(manifest, lockfile), "0.4.0"
            )

            lockfile.write_text(
                '[[package]]\nname = "tapas"\nversion = "0.3.0"\n',
                encoding="utf-8",
            )
            with self.assertRaises(ValueError):
                release_policy.repository_version(manifest, lockfile)


class CommandLineTests(unittest.TestCase):
    def test_next_version_reads_commit_subjects_from_standard_input(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            manifest, lockfile = write_version_files(root, "0.3.0")
            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPTS / "release_policy.py"),
                    "next-version",
                    "--current",
                    "v0.3.0",
                    "--manifest",
                    str(manifest),
                    "--lockfile",
                    str(lockfile),
                ],
                input="patch: repair output\nminor: add support\n",
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "0.4.0\n")

    def test_next_version_is_empty_when_release_is_already_prepared(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest, lockfile = write_version_files(
                pathlib.Path(directory), "0.4.0"
            )
            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPTS / "release_policy.py"),
                    "next-version",
                    "--current",
                    "v0.3.0",
                    "--manifest",
                    str(manifest),
                    "--lockfile",
                    str(lockfile),
                ],
                input="minor: add support\n",
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_validate_title_reports_a_useful_failure(self) -> None:
        result = subprocess.run(
            [
                "python3",
                str(SCRIPTS / "release_policy.py"),
                "validate-title",
                "Minor: malformed reserved prefix",
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("release-intent prefixes must use lowercase", result.stderr)


if __name__ == "__main__":
    unittest.main()
