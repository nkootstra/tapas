from __future__ import annotations

import json
import pathlib
import sqlite3
import sys
import tempfile
import unittest


SCRIPTS = pathlib.Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))

import usage_report  # noqa: E402


class UsageReportTests(unittest.TestCase):
    def test_normalization_removes_wrappers_and_reads_shell_scripts(self) -> None:
        self.assertEqual(
            usage_report.normalize_invocation("BUILD=1 smll --raw -- git status"),
            [("git", ["status"])],
        )
        self.assertEqual(
            usage_report.normalize_invocation("bash -lc 'cargo test --locked'"),
            [("cargo", ["test", "--locked"])],
        )
        self.assertEqual(
            usage_report.normalize_invocation("cd project && git diff"),
            [("git", ["diff"])],
        )
        self.assertEqual(
            usage_report.normalize_invocation("{ grep foo; }"),
            [("grep", ["foo"])],
        )

    def test_normalization_reads_compound_commands_without_splitting_quoted_text(self) -> None:
        self.assertEqual(
            usage_report.normalize_invocation("git status && cargo test"),
            [("git", ["status"]), ("cargo", ["test"])],
        )
        self.assertEqual(
            usage_report.normalize_invocation("git status&&cargo test"),
            [("git", ["status"]), ("cargo", ["test"])],
        )
        self.assertEqual(
            usage_report.normalize_invocation("git log --format='status && test' && cargo test"),
            [
                ("git", ["log", "--format=status && test"]),
                ("cargo", ["test"]),
            ],
        )
        self.assertEqual(
            usage_report.normalize_invocation("bash -lc 'git status && cargo test'"),
            [("git", ["status"]), ("cargo", ["test"])],
        )

    def test_collectors_read_opencode_and_jsonl_tool_calls(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            database = root / "opencode.db"
            with sqlite3.connect(database) as connection:
                connection.execute("CREATE TABLE part (data TEXT NOT NULL)")
                connection.execute(
                    "INSERT INTO part VALUES (?)",
                    (
                        json.dumps(
                            {
                                "type": "tool",
                                "tool": "bash",
                                "state": {"input": {"command": "git status"}},
                            }
                        ),
                    ),
                )
                connection.execute("INSERT INTO part VALUES (?)", ("not-json",))
                connection.commit()

            sessions = root / "sessions"
            sessions.mkdir()
            (sessions / "session.jsonl").write_text(
                json.dumps(
                    {
                        "type": "response_item",
                        "payload": {
                            "type": "function_call",
                            "arguments": json.dumps({"cmd": "cargo test --locked"}),
                        },
                    }
                )
                + "\n",
                encoding="utf-8",
            )

            self.assertEqual(
                usage_report.collect_opencode(database),
                [("opencode", "git status")],
            )
            self.assertEqual(
                usage_report.collect_jsonl(sessions, "codex"),
                [("codex", "cargo test --locked")],
            )

    def test_report_identifies_catalog_and_git_subcommand_gaps(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &["git", "cargo"];
            pub const WRAPPER_COMMANDS: &[&str] = &["git", "cargo", "node"];
            pub const GIT_SUBCOMMANDS: &[&str] = &["status"];
            pub const TRANSPARENT_RUNNERS: &[&str] = &["npx"];
            """
        )
        rows = usage_report.normalize_rows(
            [
                ("opencode", "git status"),
                ("opencode", "git status && cargo test"),
                ("opencode", "git remote -v"),
                ("codex", "mystery-tool --help"),
            ]
        )
        report = usage_report.build_report(rows, catalog)

        self.assertEqual(report["total_invocations"], 5)
        command_counts = {record["command"]: record["count"] for record in report["commands"]}
        self.assertEqual(command_counts["cargo"], 1)
        self.assertEqual(report["unlisted_commands"][0]["command"], "mystery-tool")
        git_counts = {
            record["subcommand"]: record["count"] for record in report["git_subcommands"]
        }
        self.assertEqual(git_counts["status"], 2)
        self.assertEqual(report["unlisted_git_subcommands"][0]["subcommand"], "remote")
        self.assertEqual(report["commands"][0]["coverage"], "auto-wrap")

    def test_report_includes_compaction_candidates_with_estimated_savings(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &["git", "cargo"];
            pub const WRAPPER_COMMANDS: &[&str] = &["git", "cargo"];
            pub const GIT_SUBCOMMANDS: &[&str] = &["status"];
            pub const TRANSPARENT_RUNNERS: &[&str] = &[];
            """
        )
        rows = usage_report.normalize_rows(
            [
                ("opencode", "unlisted-a task"),
                ("opencode", "unlisted-a report"),
                ("opencode", "unlisted-b task"),
                ("opencode", "git status"),
            ]
        )
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "compaction.jsonl"
            path.write_text(
                "\n".join(
                    [
                        "{\"command\":\"unlisted-a\",\"raw_bytes\":1000,\"displayed_bytes\":100,\"changed\":true}",
                        "{\"command\":\"unlisted-a\",\"raw_bytes\":500,\"diagnostic_bytes\":20,\"displayed_bytes\":50,\"changed\":false}",
                        "{\"command\":\"unlisted-b\",\"raw_bytes\":300,\"displayed_bytes\":120,\"changed\":true}",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            report = usage_report.build_report(
                rows,
                catalog,
                1,
                compaction_metrics=usage_report.load_compaction_metrics(path),
            )

        candidates = report["compaction_candidates"]
        self.assertEqual(candidates[0]["command"], "unlisted-a")
        self.assertEqual(candidates[0]["estimated_saved_bytes"], 900)
        self.assertEqual(candidates[0]["estimated_saved_tokens"], 225)
        self.assertEqual(candidates[1]["command"], "unlisted-b")
        self.assertEqual(candidates[1]["estimated_saved_bytes"], 180)

    def test_transparent_runner_requires_the_declared_subcommand(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &[];
            pub const WRAPPER_COMMANDS: &[&str] = &["uv"];
            pub const GIT_SUBCOMMANDS: &[&str] = &["status"];
            pub const TRANSPARENT_RUNNERS: &[&str] = &["uv run"];
            """
        )
        rows = usage_report.normalize_rows(
            [("opencode", "uv run pytest"), ("opencode", "uv pip install ruff")]
        )
        report = usage_report.build_report(rows, catalog)
        coverage = {record["command"]: record["coverage"] for record in report["commands"]}
        self.assertEqual(coverage["uv"], "mixed")
        self.assertEqual(report["unlisted_commands"], [])


if __name__ == "__main__":
    unittest.main()
