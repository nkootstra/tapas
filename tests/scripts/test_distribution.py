from __future__ import annotations

import json
import os
import pathlib
import subprocess
import tempfile
import textwrap
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


def workflow_run_blocks(workflow: str) -> list[str]:
    lines = workflow.splitlines()
    blocks = []
    index = 0
    while index < len(lines):
        line = lines[index]
        if line.strip() != "run: |":
            index += 1
            continue
        key_indent = len(line) - len(line.lstrip())
        index += 1
        block = []
        while index < len(lines):
            candidate = lines[index]
            indent = len(candidate) - len(candidate.lstrip())
            if candidate.strip() and indent <= key_indent:
                break
            block.append(candidate)
            index += 1
        blocks.append(textwrap.dedent("\n".join(block)).strip())
    return blocks


class DistributionTests(unittest.TestCase):
    def test_pr_installer_rejects_failed_downloads_without_running_partial_scripts(self) -> None:
        for body in ("", "echo partial-installer-ran\n"):
            with self.subTest(body=body), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                curl = root / "curl"
                curl.write_text(
                    "#!/bin/sh\nprintf '%s' '" + body + "'\nexit 22\n",
                    encoding="utf-8",
                )
                curl.chmod(0o755)
                result = subprocess.run(
                    ["sh", str(ROOT / "install-pr.sh"), "123"],
                    env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}", "TMPDIR": directory},
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=10,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")
                self.assertEqual(sorted(path.name for path in root.iterdir()), ["curl"])

    def test_pr_installer_preserves_arguments_and_installer_exit_status(self) -> None:
        for status in (0, 17):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                curl = root / "curl"
                curl.write_text(
                    "#!/bin/sh\ncat <<'SCRIPT'\n"
                    'printf "<%s>\\n" "$@"\n'
                    f"exit {status}\nSCRIPT\n",
                    encoding="utf-8",
                )
                curl.chmod(0o755)
                result = subprocess.run(
                    ["sh", str(ROOT / "install-pr.sh"), "123", "argument with spaces"],
                    env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}", "TMPDIR": directory},
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=10,
                )
                self.assertEqual(result.returncode, status, result.stderr)
                self.assertEqual(result.stdout, "<--pr>\n<123>\n<argument with spaces>\n")
                self.assertEqual(sorted(path.name for path in root.iterdir()), ["curl"])

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

    def test_windows_cleanup_script_is_scoped_and_documented(self) -> None:
        script = (ROOT / "install.ps1").read_text(encoding="utf-8")
        self.assertIn("CleanDevBuilds", script)
        self.assertIn("DryRun", script)
        self.assertIn("tapas-pr-*", script)
        self.assertIn("Remove-Item -LiteralPath", script)
        self.assertNotIn("Expand-Archive", script)

    def test_install_script_cleans_only_local_pr_builds(self) -> None:
        script = ROOT / "install.sh"
        with tempfile.TemporaryDirectory() as directory:
            install_dir = pathlib.Path(directory)
            stable = install_dir / "tapas"
            dev_one = install_dir / "tapas-pr-11111111"
            dev_two = install_dir / "tapas-pr-22222222"
            unrelated = install_dir / "other-tool"
            for path in (stable, dev_one, dev_two, unrelated):
                path.write_text("binary", encoding="utf-8")

            environment = {**os.environ, "TAPAS_INSTALL_DIR": str(install_dir)}
            dry_run = subprocess.run(
                ["sh", str(script), "--clean-dev-builds", "--dry-run"],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(dry_run.returncode, 0, dry_run.stderr)
            self.assertIn(str(dev_one), dry_run.stdout)
            self.assertTrue(dev_one.exists())

            cleaned = subprocess.run(
                ["sh", str(script), "--clean-dev-builds"],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(cleaned.returncode, 0, cleaned.stderr)
            self.assertFalse(dev_one.exists())
            self.assertFalse(dev_two.exists())
            self.assertTrue(stable.exists())
            self.assertTrue(unrelated.exists())

    def test_package_metadata_records_the_build_label(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            binary = root / "tapas"
            output = root / "dist"
            binary.write_bytes(b"fake executable")
            result = subprocess.run(
                [
                    "python3",
                    str(ROOT / "scripts/package_artifact.py"),
                    "--binary",
                    str(binary),
                    "--output",
                    str(output),
                    "--binary-name",
                    "tapas.exe",
                    "--version",
                    "0.1.0",
                    "--version-label",
                    "0.1.0-dev.12345678",
                    "--source-sha",
                    "0123456789abcdef0123456789abcdef01234567",
                    "--target",
                    "test-target",
                    "--abi",
                    "test",
                    "--workflow-run",
                    "123",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            metadata = json.loads((output / "BUILD-METADATA.json").read_text(encoding="utf-8"))
            self.assertTrue((output / "tapas.exe").is_file())
            self.assertEqual(metadata["version"], "0.1.0")
            self.assertEqual(metadata["version_label"], "0.1.0-dev.12345678")
            self.assertEqual(metadata["binary"]["name"], "tapas.exe")
            checksums = (output / "SHA256SUMS").read_text(encoding="ascii").splitlines()
            self.assertEqual(
                checksums[0],
                "7a69b7f9aa0f0693d048b5c58d321f6409aa34ef50f044362d71b9b98828a6db  tapas.exe",
            )

    def test_workflows_keep_privileged_operations_out_of_pr_code(self) -> None:
        publisher = (ROOT / ".github/workflows/publish-pr.yml").read_text(encoding="utf-8")
        release = (ROOT / ".github/workflows/publish-release.yml").read_text(encoding="utf-8")
        cleanup = (ROOT / ".github/workflows/cleanup-pr-builds.yml").read_text(encoding="utf-8")
        self.assertIn("workflow_run", publisher)
        self.assertNotIn("TAPAS_VERSION: 0.3.0", publisher)
        self.assertIn("pr-build-metadata-validator:start", publisher)
        self.assertIn("${TAPAS_BUILD_LABEL}", publisher)
        self.assertIn(r"\`\`\`sh", publisher)
        self.assertIn("install-pr.sh", publisher)
        self.assertIn("--clean-dev-builds", publisher)
        self.assertIn("--dry-run", publisher)
        self.assertIn("main_sha", publisher)
        self.assertIn("pinned_installer_url", publisher)
        self.assertIn("install.ps1", publisher)
        self.assertIn("CleanDevBuilds", publisher)
        self.assertIn("pinned_windows_installer_url", publisher)
        self.assertIn(r"^v[0-9]+\.[0-9]+\.[0-9]+$", release)
        self.assertIn('test "$version" = "${TAG#v}"', release)
        blocks = workflow_run_blocks(cleanup)
        self.assertEqual(len(blocks), 2)
        for block in blocks:
            result = subprocess.run(
                ["bash", "-n"],
                input=block,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("contents: write", publisher)
        self.assertIn("pull-requests: write", publisher)
        self.assertIn("contents: write", release)
        self.assertIn('gh api "repos/${REPOSITORY}/commits/${TAG}" --jq .sha', release)
        self.assertIn('test "$tag_sha" = "$SOURCE_SHA"', release)
        self.assertIn("git verify-tag", release)
        self.assertIn("pull_request_target", cleanup)
        self.assertNotIn("actions/checkout", publisher)
        self.assertIn("actions/checkout", release)
        self.assertNotIn("actions/checkout", cleanup)

    def test_pr_install_comment_preserves_the_pinned_windows_url(self) -> None:
        workflow = (ROOT / ".github/workflows/publish-pr.yml").read_text(encoding="utf-8")
        block = next(block for block in workflow_run_blocks(workflow) if "cat > comment.md" in block)
        environment = {
            **os.environ,
            "REPOSITORY": "example/tapas",
            "SOURCE_SHA": "a" * 40,
            "PR_NUMBER": "42",
            "TAPAS_BUILD_LABEL": "0.1.0-dev.aaaaaaaa",
        }
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(
                ["bash", "-c", 'gh() { if [[ "$*" == *"/commits/main"* ]]; then printf "%040d\\n" 0; fi; }\n' + block],
                cwd=directory,
                env=environment,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            comment = (pathlib.Path(directory) / "comment.md").read_text(encoding="utf-8")
            self.assertIn(
                "If the Windows script URL is cached, replace it with "
                "`https://github.com/example/tapas/raw/0000000000000000000000000000000000000000/install.ps1`.",
                comment,
            )
            self.assertEqual(result.stderr, "")

    def test_pr_release_is_keyed_by_head_commit(self) -> None:
        publisher = (ROOT / ".github/workflows/publish-pr.yml").read_text(encoding="utf-8")
        self.assertIn('tag="pr-${PR_NUMBER}-${SOURCE_SHA}"', publisher)
        self.assertIn("source_sha", publisher)


if __name__ == "__main__":
    unittest.main()
