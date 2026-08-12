from __future__ import annotations

import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class ReleaseAutomationContractTests(unittest.TestCase):
    def test_release_plz_prepares_release_prs_without_creating_tags(self) -> None:
        config = (ROOT / "release-plz.toml").read_text(encoding="utf-8")

        self.assertIn("git_only = true", config)
        self.assertIn("git_tag_enable = false", config)
        self.assertIn("git_release_enable = false", config)
        self.assertIn('release_commits = "^(major|minor|patch):"', config)
        self.assertIn('custom_minor_increment_regex = "^minor$"', config)
        self.assertIn('custom_major_increment_regex = "^major$"', config)
        self.assertIn('message = "^skip:"', config)
        self.assertIn("skip = true", config)

    def test_merged_release_pr_delegates_to_the_shared_tag_workflow(self) -> None:
        workflow = (ROOT / ".github/workflows/release-plz.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("types: [closed]", workflow)
        self.assertIn("github.event.pull_request.merged == true", workflow)
        self.assertIn("github.event.pull_request.user.type == 'Bot'", workflow)
        self.assertIn("uses: ./.github/workflows/release-tag.yml", workflow)
        self.assertIn(
            "release_pr_number: ${{ github.event.pull_request.number }}", workflow
        )
        self.assertIn(
            "RELEASE_APP_PRIVATE_KEY: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}",
            workflow,
        )
        self.assertIn(
            "RELEASE_SIGNING_KEY: ${{ secrets.RELEASE_SIGNING_KEY }}", workflow
        )
        self.assertNotIn("command: release\n", workflow)
        self.assertIn("command: release-pr\n", workflow)
        self.assertNotIn("pull_request_target", workflow)
        self.assertIn("group: release-automation-${{ github.event_name }}-${{ github.ref }}", workflow)
        self.assertIn("cancel-in-progress: false", workflow)

    def test_shared_tag_workflow_validates_before_writing_the_signing_key(self) -> None:
        workflow = (ROOT / ".github/workflows/release-tag.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("workflow_call:", workflow)
        self.assertIn("release_pr_number:", workflow)
        self.assertIn("type: number", workflow)
        self.assertIn("RELEASE_APP_PRIVATE_KEY:", workflow)
        self.assertIn("RELEASE_SIGNING_KEY:", workflow)
        self.assertIn("app-id: ${{ vars.RELEASE_APP_ID }}", workflow)
        self.assertIn("permissions: {}", workflow)
        self.assertIn("group: tapas-signed-release-tag", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn("permission-contents: write", workflow)
        self.assertIn("permission-pull-requests: read", workflow)
        self.assertIn("ref: ${{ github.workflow_sha }}", workflow)
        self.assertIn("fetch-depth: 0", workflow)
        self.assertIn("persist-credentials: false", workflow)
        self.assertIn("scripts/release_tag.py validate", workflow)
        self.assertIn("scripts/release_tag.py tag", workflow)
        self.assertIn(".github/release-signers", workflow)
        self.assertIn("install -m 600", workflow)
        self.assertIn("GIT_ASKPASS", workflow)
        self.assertIn("if: always()", workflow)
        self.assertIn("rm -f", workflow)

        validate = workflow.index("scripts/release_tag.py validate")
        signing_key = workflow.index('printf \'%s\\n\' "$RELEASE_SIGNING_KEY"')
        tag = workflow.index("scripts/release_tag.py tag")
        self.assertLess(validate, signing_key)
        self.assertLess(signing_key, tag)
        self.assertNotIn("git switch", workflow)
        self.assertNotIn("git checkout", workflow)
        self.assertNotIn("merge_commit_sha }}", workflow)

    def test_recovery_is_an_owner_gated_default_branch_dispatch(self) -> None:
        workflow = (ROOT / ".github/workflows/release-recovery.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("repository_dispatch:", workflow)
        self.assertIn("types: [release-recovery]", workflow)
        self.assertIn("github.actor == github.repository_owner", workflow)
        self.assertIn("uses: ./.github/workflows/release-tag.yml", workflow)
        self.assertIn(
            "release_pr_number: ${{ github.event.client_payload.release_pr_number }}",
            workflow,
        )
        self.assertNotIn("workflow_dispatch:", workflow)
        self.assertNotIn("pull_request_target:", workflow)

    def test_prepare_token_is_narrow_and_never_receives_the_signing_key(self) -> None:
        workflow = (ROOT / ".github/workflows/release-plz.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("permission-contents: write", workflow)
        self.assertIn("permission-pull-requests: write", workflow)
        self.assertEqual(
            workflow.count(
                "RELEASE_SIGNING_KEY: ${{ secrets.RELEASE_SIGNING_KEY }}"
            ),
            1,
        )
        fallback = workflow.index("Apply literal SemVer to the release PR")
        prepare = re.search(
            r"(?ms)^  prepare:\n.*?(?=^  [A-Za-z0-9_-]+:\n|\Z)", workflow
        )
        self.assertIsNotNone(prepare)
        self.assertNotIn("RELEASE_SIGNING_KEY", prepare.group(0))
        self.assertIn("createCommitOnBranch", workflow[fallback:])
        self.assertNotIn("commit -S", workflow[fallback:])
        self.assertNotIn("if: steps.release-plz.outputs.prs_created", workflow)
        self.assertIn("release_normalization.py inspect", workflow[fallback:])
        self.assertIn("release_normalization.py mutation", workflow[fallback:])
        self.assertIn("release_normalization.py body", workflow[fallback:])

    def test_all_actions_are_pinned_and_release_shells_do_not_interpolate_events(self) -> None:
        workflows = list((ROOT / ".github/workflows").glob("*.yml"))
        self.assertGreater(len(workflows), 0)
        for path in workflows:
            contents = path.read_text(encoding="utf-8")
            action_refs = re.findall(r"uses:\s+[^./\s][^@\s]*@([^\s]+)", contents)
            self.assertTrue(
                all(re.fullmatch(r"[0-9a-f]{40}", ref) for ref in action_refs), path
            )

        for filename in (
            "release-plz.yml",
            "release-tag.yml",
            "release-recovery.yml",
        ):
            contents = (ROOT / ".github/workflows" / filename).read_text(
                encoding="utf-8"
            )
            run_blocks = re.findall(
                r"^\s+run:\s*\|\n((?:^\s{8,}.*\n?)*)", contents, re.MULTILINE
            )
            self.assertTrue(
                all("${{ github.event" not in block for block in run_blocks), filename
            )

    def test_release_policy_validates_the_actual_merge_title(self) -> None:
        workflow = (ROOT / ".github/workflows/release-plz.yml").read_text(
            encoding="utf-8"
        )
        guide = (ROOT / "docs/github-app-release-automation.md").read_text(
            encoding="utf-8"
        )

        self.assertIn('subject="$(git log -1 --format=%s)"', workflow)
        self.assertIn('release_policy.py validate-title "$subject"', workflow)
        self.assertNotIn('gh api "repos/${GITHUB_REPOSITORY}"', workflow)
        self.assertIn("Always use the pull request title", guide)
        self.assertIn("These repository settings are required setup", guide)
        self.assertIn(
            "The workflow does not inspect the repository merge settings at runtime",
            guide,
        )
        self.assertIn("it validates the actual merged commit subject", guide)
        self.assertIn("fails closed unless that subject starts with", guide)
        self.assertNotIn(
            "The release workflow checks these settings before calculating a version",
            guide,
        )
        self.assertIn("gh api --method PATCH repos/nkootstra/tapas", guide)

    def test_title_workflow_validates_inline_without_checking_out_pr_code(self) -> None:
        workflow = (ROOT / ".github/workflows/pr-title.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("pull_request:", workflow)
        self.assertIn("major|minor|patch|skip", workflow)
        self.assertNotIn("actions/checkout", workflow)
        self.assertNotIn("pull_request_target", workflow)

    def test_ci_and_publisher_verify_release_tag_signatures(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        publisher = (ROOT / ".github/workflows/publish-release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("git verify-tag", ci)
        self.assertIn("git verify-tag", publisher)
        self.assertIn(".github/release-signers", ci)
        self.assertIn(".github/release-signers", publisher)
        self.assertIn("github.event.repository.default_branch", ci)
        self.assertIn(
            'refs/remotes/origin/trusted-release-policy:.github/release-signers',
            ci,
        )
        self.assertIn('$RUNNER_TEMP/release-signers', ci)
        self.assertNotIn(
            'gpg.ssh.allowedSignersFile ".github/release-signers"',
            ci,
        )
        self.assertIn("github.event.repository.default_branch", publisher)

    def test_operator_guide_names_every_required_setting_without_private_material(self) -> None:
        guide = (ROOT / "docs/github-app-release-automation.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("RELEASE_APP_ID", guide)
        self.assertIn("RELEASE_APP_PRIVATE_KEY", guide)
        self.assertIn("RELEASE_SIGNING_KEY", guide)
        self.assertNotIn("BEGIN OPENSSH PRIVATE KEY", guide)
        self.assertNotIn("BEGIN RSA PRIVATE KEY", guide)

    def test_release_policy_trusts_one_non_compromised_ed25519_key(self) -> None:
        signers = (ROOT / ".github/release-signers").read_text(encoding="utf-8")
        lines = [line for line in signers.splitlines() if line]

        self.assertEqual(len(lines), 1)
        self.assertRegex(
            lines[0],
            r'^545768\+nkootstra@users\.noreply\.github\.com '
            r'namespaces="git" ssh-ed25519 [A-Za-z0-9+/]+={0,2}$',
        )
        self.assertNotIn(
            "AAAAC3NzaC1lZDI1NTE5AAAAILnWIno+oW9pcQkzKEWQuxo6/"
            "OvZHPtrxY+P1FL4qiQr",
            signers,
        )

    def test_operator_guide_documents_direct_tag_recovery_and_hard_rotation(self) -> None:
        guide = (ROOT / "docs/github-app-release-automation.md").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            "release-plz prepares version and changelog pull requests", guide
        )
        self.assertIn("Tapas creates the signed release tag", guide)
        self.assertIn("Merging a release PR is the release approval", guide)
        self.assertIn("repository_dispatch", guide)
        self.assertIn("github.actor == github.repository_owner", guide)
        self.assertIn(
            "-F 'client_payload[release_pr_number]=15'",
            guide,
        )
        self.assertIn("missing tag", guide)
        self.assertIn("valid replay is a no-op", guide)
        self.assertIn("rerun the failed downstream workflow", guide)
        self.assertIn("fix forward with a patch release", guide)
        self.assertIn("Replace, rather than append to", guide)
        self.assertIn("Revoke the exposed GitHub signing key", guide)
        self.assertIn("delete its old local private and public key files", guide)
        self.assertIn("Contents: Read and write", guide)
        self.assertIn("Pull requests: Read and write", guide)
        self.assertIn("GraphQL commits show as Verified", guide)
        self.assertIn("GitHub-signed normalization fallback", guide)


if __name__ == "__main__":
    unittest.main()
