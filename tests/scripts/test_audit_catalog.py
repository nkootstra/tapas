from __future__ import annotations

import pathlib
import sys
import unittest


SCRIPTS = pathlib.Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))

import audit_catalog  # noqa: E402


class CatalogAuditTests(unittest.TestCase):
    def test_real_catalog_and_git_dispatch_parse_as_owned(self) -> None:
        source = audit_catalog.CATALOG.read_text(encoding="utf-8")
        self.assertIn("sqlite3", audit_catalog.parse_const(source, "AUTO_WRAP_COMMANDS"))
        git_source = audit_catalog.GIT.read_text(encoding="utf-8")
        git_dispatch = audit_catalog.parse_git_dispatch(git_source)
        self.assertTrue({"config", "push", "tag", "worktree"} <= git_dispatch)

    def test_audit_reports_missing_catalog_ownership_and_dispatch(self) -> None:
        errors = audit_catalog.check_catalog(
            auto_wrap=["missing-command"],
            wrapper=[],
            git_subcommands=["missing-subcommand"],
        )
        self.assertTrue(any("AUTO_WRAP_COMMANDS" in error for error in errors))
        self.assertTrue(any("git subcommands without" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
