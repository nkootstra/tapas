from __future__ import annotations

import pathlib
import subprocess
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class DistributionTests(unittest.TestCase):
    def test_install_scripts_have_valid_shell_syntax(self) -> None:
        for name in ("install.sh", "install-pr.sh"):
            result = subprocess.run(
                ["sh", "-n", str(ROOT / name)],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_install_script_supports_stable_and_pr_modes(self) -> None:
        script = (ROOT / "install.sh").read_text(encoding="utf-8")
        self.assertIn("--pr", script)
        self.assertIn("--version", script)
        self.assertIn("BUILD-METADATA.json", script)
        self.assertIn("source_sha", script)
        self.assertIn("SHA256SUMS", script)

    def test_workflows_keep_privileged_operations_out_of_pr_code(self) -> None:
        publisher = (ROOT / ".github/workflows/publish-pr.yml").read_text(encoding="utf-8")
        release = (ROOT / ".github/workflows/publish-release.yml").read_text(encoding="utf-8")
        cleanup = (ROOT / ".github/workflows/cleanup-pr-builds.yml").read_text(encoding="utf-8")
        self.assertIn("workflow_run", publisher)
        self.assertIn("contents: write", publisher)
        self.assertIn("pull-requests: write", publisher)
        self.assertIn("contents: write", release)
        self.assertIn("pull_request_target", cleanup)
        self.assertNotIn("actions/checkout", publisher)
        self.assertNotIn("actions/checkout", release)
        self.assertNotIn("actions/checkout", cleanup)

    def test_pr_release_is_keyed_by_head_commit(self) -> None:
        publisher = (ROOT / ".github/workflows/publish-pr.yml").read_text(encoding="utf-8")
        self.assertIn('tag="pr-${PR_NUMBER}-${SOURCE_SHA}"', publisher)
        self.assertIn("source_sha", publisher)


if __name__ == "__main__":
    unittest.main()
