from __future__ import annotations

import pathlib
import subprocess
import sys
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))

import release_policy  # noqa: E402


class PullRequestTitleTests(unittest.TestCase):
    def test_accepts_release_and_skip_titles(self) -> None:
        for title in (
            "major: remove the legacy protocol",
            "minor: add release automation",
            "patch: repair tag validation",
            "skip: update internal documentation",
        ):
            with self.subTest(title=title):
                self.assertEqual(release_policy.validate_title(title), None)

    def test_rejects_titles_outside_the_release_contract(self) -> None:
        for title in (
            "feat: add release automation",
            "Major: remove the legacy protocol",
            "patch:no separating space",
            "skip: ",
            "minor: first line\nsecond line",
        ):
            with self.subTest(title=title):
                with self.assertRaises(ValueError):
                    release_policy.validate_title(title)


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


class CommandLineTests(unittest.TestCase):
    def test_next_version_reads_commit_subjects_from_standard_input(self) -> None:
        result = subprocess.run(
            [
                "python3",
                str(SCRIPTS / "release_policy.py"),
                "next-version",
                "--current",
                "v0.3.0",
            ],
            input="patch: repair output\nminor: add support\n",
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "0.4.0\n")

    def test_validate_title_reports_a_useful_failure(self) -> None:
        result = subprocess.run(
            [
                "python3",
                str(SCRIPTS / "release_policy.py"),
                "validate-title",
                "feat: unsupported title",
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("major:, minor:, patch:, or skip:", result.stderr)


if __name__ == "__main__":
    unittest.main()
