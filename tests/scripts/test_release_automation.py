from __future__ import annotations

import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class ReleaseAutomationContractTests(unittest.TestCase):
    def test_release_plz_uses_git_tags_and_leaves_release_publishing_to_tapas(self) -> None:
        config = (ROOT / "release-plz.toml").read_text(encoding="utf-8")

        self.assertIn("git_only = true", config)
        self.assertIn("git_tag_enable = true", config)
        self.assertIn("git_release_enable = false", config)
        self.assertIn('release_commits = "^(major|minor|patch):"', config)
        self.assertIn('custom_minor_increment_regex = "^minor$"', config)
        self.assertIn('custom_major_increment_regex = "^major$"', config)
        self.assertIn('message = "^skip:"', config)
        self.assertIn("skip = true", config)

    def test_release_workflow_uses_app_authentication_and_signed_tags(self) -> None:
        workflow = (ROOT / ".github/workflows/release-plz.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("RELEASE_APP_ID", workflow)
        self.assertIn("RELEASE_APP_PRIVATE_KEY", workflow)
        self.assertIn("RELEASE_SIGNING_KEY", workflow)
        self.assertIn("tag.gpgSign true", workflow)
        self.assertIn("commit.gpgSign true", workflow)
        self.assertIn("git verify-tag", workflow)
        self.assertNotIn("pull_request_target", workflow)

        action_refs = re.findall(r"uses:\s+[^@\s]+@([^\s]+)", workflow)
        self.assertGreaterEqual(len(action_refs), 3)
        self.assertTrue(all(re.fullmatch(r"[0-9a-f]{40}", ref) for ref in action_refs))

    def test_release_and_pr_preparation_use_separate_concurrency_policy(self) -> None:
        workflow = (ROOT / ".github/workflows/release-plz.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("command: release\n", workflow)
        self.assertIn("command: release-pr\n", workflow)
        self.assertNotIn("workflow_dispatch:", workflow)
        self.assertEqual(workflow.count("concurrency:"), 1)
        self.assertIn("group: release-plz-pr-${{ github.ref }}", workflow)

    def test_signed_release_requires_a_merged_app_authored_release_pr(self) -> None:
        workflow = (ROOT / ".github/workflows/release-plz.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("types: [closed]", workflow)
        self.assertIn("github.event.pull_request.merged == true", workflow)
        self.assertIn("github.event.pull_request.user.type == 'Bot'", workflow)
        self.assertIn(
            "startsWith(github.event.pull_request.head.ref, 'release-plz-')",
            workflow,
        )
        self.assertIn(
            "startsWith(github.event.pull_request.title, 'skip: prepare v')",
            workflow,
        )
        self.assertIn("steps.app-token.outputs.app-slug", workflow)
        self.assertIn("github.event.pull_request.user.login", workflow)
        self.assertIn("github.event.pull_request.merge_commit_sha", workflow)
        self.assertIn('test "$PR_AUTHOR" = "${APP_SLUG}[bot]"', workflow)

    def test_release_policy_fails_closed_when_squash_titles_can_drift(self) -> None:
        workflow = (ROOT / ".github/workflows/release-plz.yml").read_text(
            encoding="utf-8"
        )
        guide = (ROOT / "docs/github-app-release-automation.md").read_text(
            encoding="utf-8"
        )

        self.assertIn('.squash_merge_commit_title == "PR_TITLE"', workflow)
        self.assertIn(".allow_merge_commit == false", workflow)
        self.assertIn(".allow_rebase_merge == false", workflow)
        self.assertIn("Always use the pull request title", guide)

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


if __name__ == "__main__":
    unittest.main()
