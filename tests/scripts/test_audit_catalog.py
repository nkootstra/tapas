from __future__ import annotations

import pathlib
import sys
import unittest


SCRIPTS = pathlib.Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))

import audit_catalog  # noqa: E402
import catalog_source  # noqa: E402


class CatalogAuditTests(unittest.TestCase):
    def test_real_catalog_and_git_dispatch_parse_as_owned(self) -> None:
        source = audit_catalog.CATALOG.read_text(encoding="utf-8")
        self.assertIn("sqlite3", audit_catalog.parse_const(source, "AUTO_WRAP_COMMANDS"))
        git_source = audit_catalog.GIT.read_text(encoding="utf-8")
        git_dispatch = audit_catalog.parse_git_dispatch(git_source)
        self.assertTrue({"config", "push", "tag", "worktree"} <= git_dispatch)
        families = catalog_source.parse_filter_families(source)
        self.assertTrue({"build", "test_tools"} <= {
            family for family, commands in families.items() if "cargo" in commands
        })
        self.assertTrue({"data", "infra"} <= {
            family for family, commands in families.items() if "docker" in commands
        })

    def test_audit_reports_missing_catalog_ownership(self) -> None:
        stream_filters = audit_catalog.parse_stream_filters(
            audit_catalog.PROCESS.read_text(encoding="utf-8")
        )
        errors = audit_catalog.check_catalog(
            auto_wrap=["missing-command"],
            wrapper=["missing-command"],
            git_subcommands=[],
            transparent_runners=["missing-command"],
            filter_families={family: set() for family in stream_filters},
            filter_family_exemptions=set(),
            stream_filters=stream_filters,
        )
        self.assertEqual(
            ["auto-wrap commands with no filter family: missing-command"], errors
        )

    def test_audit_reports_auto_wrap_command_missing_from_wrappers(self) -> None:
        stream_filters = audit_catalog.parse_stream_filters(
            audit_catalog.PROCESS.read_text(encoding="utf-8")
        )
        filter_families = {family: set() for family in stream_filters}
        filter_families["build"].add("missing-command")
        errors = audit_catalog.check_catalog(
            auto_wrap=["missing-command"],
            wrapper=[],
            git_subcommands=[],
            transparent_runners=[],
            filter_families=filter_families,
            filter_family_exemptions=set(),
            stream_filters=stream_filters,
        )
        self.assertEqual(
            ["AUTO_WRAP_COMMANDS missing from WRAPPER_COMMANDS: missing-command"],
            errors,
        )

    def test_audit_reports_missing_git_dispatch(self) -> None:
        stream_filters = audit_catalog.parse_stream_filters(
            audit_catalog.PROCESS.read_text(encoding="utf-8")
        )
        errors = audit_catalog.check_catalog(
            auto_wrap=[],
            wrapper=[],
            git_subcommands=["missing-subcommand"],
            transparent_runners=[],
            filter_families={family: set() for family in stream_filters},
            filter_family_exemptions=set(),
            stream_filters=stream_filters,
        )
        self.assertIn(
            "git subcommands without a dispatch arm in git.rs: missing-subcommand",
            errors,
        )

    def test_audit_reports_mismatched_handler_catalog_constant(self) -> None:
        source = (audit_catalog.FILTERS_DIR / "build.rs").read_text(encoding="utf-8")
        mutated = source.replace("BUILD_FILTER_COMMANDS", "DATA_FILTER_COMMANDS")
        self.assertEqual(
            "filter family build handles_argv does not reference BUILD_FILTER_COMMANDS",
            audit_catalog.check_handler_wiring("build", mutated),
        )

    def test_audit_reports_mismatched_stream_registry_handler(self) -> None:
        source = audit_catalog.PROCESS.read_text(encoding="utf-8")
        mutated = source.replace(
            "handles: crate::filters::build::handles_argv",
            "handles: crate::filters::data::handles_argv",
        )
        stream_filters = audit_catalog.parse_stream_filters(mutated)
        errors = audit_catalog.check_catalog(
            auto_wrap=[],
            wrapper=[],
            git_subcommands=[],
            transparent_runners=[],
            filter_families={family: set() for family in stream_filters},
            filter_family_exemptions=set(),
            stream_filters=stream_filters,
        )
        self.assertIn(
            "stream filter registry handler mismatch for build: expected "
            "crate::filters::build::handles_argv, found "
            "crate::filters::data::handles_argv",
            errors,
        )


if __name__ == "__main__":
    unittest.main()
