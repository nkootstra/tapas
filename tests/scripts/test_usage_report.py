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
                            "name": "exec_command",
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

    def test_jsonl_extraction_ignores_command_text_outside_known_tool_calls(self) -> None:
        records = [
            {
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "input": "const example = {'command': 'npm test'};",
                },
            },
            {
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "input": "SELECT '{\"cmd\": \"cargo test\"}' AS example;",
                },
            },
            {
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "database_query",
                    "arguments": json.dumps({"command": "npm test"}),
                },
            },
        ]

        commands = [
            command
            for record in records
            for command in usage_report.commands_from_json_line(json.dumps(record))
        ]

        self.assertEqual(commands, [])

    def test_jsonl_extraction_reads_known_shell_tool_envelopes(self) -> None:
        records = [
            {
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "name": "Bash",
                            "input": {"command": "git status"},
                        }
                    ]
                },
            },
            {
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "name": "functions.exec",
                    "input": 'await tools.exec_command({"cmd": "cargo test"})',
                },
            },
        ]

        commands = [
            command
            for record in records
            for command in usage_report.commands_from_json_line(json.dumps(record))
        ]

        self.assertEqual(commands, ["git status", "cargo test"])

    def test_report_identifies_catalog_and_git_subcommand_gaps(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &["git", "cargo"];
            pub const WRAPPER_COMMANDS: &[&str] = &["git", "cargo", "node"];
            pub const GIT_SUBCOMMANDS: &[&str] = &["status"];
            pub const TRANSPARENT_RUNNERS: &[&str] = &["npx"];
            pub const COMPACT_ROUTES: &[&str] = &["git:git_status", "pytest:pytest"];
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

    def test_effective_report_distinguishes_routing_from_compaction(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &["cargo", "vite"];
            pub const WRAPPER_COMMANDS: &[&str] = &["cargo", "vite"];
            pub const GIT_SUBCOMMANDS: &[&str] = &[];
            pub const TRANSPARENT_RUNNERS: &[&str] = &[];
            pub const COMPACT_ROUTES: &[&str] = &["vite:vite_build"];
            """
        )
        report = usage_report.build_report(
            usage_report.normalize_rows(
                [("codex", "cargo build"), ("codex", "vite build")]
            ),
            catalog,
        )
        coverage = {
            row["command"]: row["compaction_coverage"]
            for row in report["effective_commands"]
        }
        self.assertEqual(
            coverage,
            {"cargo": "route-undeclared", "vite": "catalogued-route"},
        )

    def test_report_aggregates_effective_commands_and_runner_chains(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &["git", "pytest"];
            pub const WRAPPER_COMMANDS: &[&str] = &["git", "npx"];
            pub const GIT_SUBCOMMANDS: &[&str] = &["status"];
            pub const TRANSPARENT_RUNNERS: &[&str] = &["npx"];
            pub const COMPACT_ROUTES: &[&str] = &["git:git_status", "pytest:pytest"];
            """
        )
        rows = usage_report.normalize_rows(
            [("codex", "git status"), ("codex", "npx pytest -q")]
        )

        report = usage_report.build_report(rows, catalog)

        self.assertEqual(
            report["effective_commands"],
            [
                {
                    "command": "git",
                    "count": 1,
                    "routing_coverage": "auto-wrap",
                    "compaction_coverage": "catalogued-route",
                    "runtime_dispatchable_count": 1,
                    "runner_chains": [],
                },
                {
                    "command": "pytest",
                    "count": 1,
                    "routing_coverage": "transparent-runner",
                    "compaction_coverage": "catalogued-route",
                    "runtime_dispatchable_count": 1,
                    "runner_chains": [{"chain": ["npx"], "count": 1}],
                },
            ],
        )
        self.assertEqual(
            report["runner_chains"],
            [{"chain": ["npx"], "count": 1, "runtime_dispatchable_count": 1}],
        )
        self.assertEqual(report["unlisted_effective_commands"], [])

    def test_runner_reporting_matches_runtime_options_and_supports_four_layers(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &["pytest"];
            pub const WRAPPER_COMMANDS: &[&str] = &["npx", "uv"];
            pub const GIT_SUBCOMMANDS: &[&str] = &[];
            pub const TRANSPARENT_RUNNERS: &[&str] = &["npx", "uv run"];
            """
        )
        rows = usage_report.normalize_rows(
            [
                ("codex", "uv run --project repo --offline -- pytest -q"),
                ("codex", "npx --future pytest"),
                ("codex", "npx uv run pytest"),
            ]
        )

        report = usage_report.build_report(rows, catalog)
        effective = {record["command"]: record for record in report["effective_commands"]}

        self.assertEqual(effective["pytest"]["runtime_dispatchable_count"], 2)
        self.assertEqual(
            effective["pytest"]["runner_chains"],
            [
                {"chain": ["npx", "uv run"], "count": 1},
                {"chain": ["uv run"], "count": 1},
            ],
        )
        self.assertEqual(effective["npx"]["routing_coverage"], "wrapper-only")
        self.assertEqual(effective["npx"]["runtime_dispatchable_count"], 0)
        self.assertIn(
            {"chain": ["npx"], "count": 1, "runtime_dispatchable_count": 0},
            report["runner_chains"],
        )

    def test_runner_reporting_fails_closed_after_four_layers(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &["pytest"];
            pub const WRAPPER_COMMANDS: &[&str] = &["npx"];
            pub const GIT_SUBCOMMANDS: &[&str] = &[];
            pub const TRANSPARENT_RUNNERS: &[&str] = &["npx"];
            """
        )
        rows = usage_report.normalize_rows(
            [
                ("codex", "npx npx npx npx pytest"),
                ("codex", "npx npx npx npx npx pytest"),
            ]
        )

        report = usage_report.build_report(rows, catalog)
        effective = {record["command"]: record for record in report["effective_commands"]}

        self.assertEqual(effective["pytest"]["runtime_dispatchable_count"], 1)
        self.assertEqual(effective["npx"]["runtime_dispatchable_count"], 0)
        self.assertIn(
            {
                "chain": ["npx", "npx", "npx", "npx", "npx"],
                "count": 1,
                "runtime_dispatchable_count": 0,
            },
            report["runner_chains"],
        )

    def test_runner_reporting_rejects_unsupported_combined_short_flags(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &[];
            pub const WRAPPER_COMMANDS: &[&str] = &["poetry"];
            pub const GIT_SUBCOMMANDS: &[&str] = &[];
            pub const TRANSPARENT_RUNNERS: &[&str] = &["poetry run"];
            """
        )
        rows = usage_report.normalize_rows([("codex", "poetry -qx run pytest")])

        report = usage_report.build_report(rows, catalog)

        self.assertEqual(report["effective_commands"][0]["command"], "poetry")
        self.assertEqual(
            report["effective_commands"][0]["runtime_dispatchable_count"], 0
        )

    def test_unknown_commands_remain_unlisted_through_transparent_runners(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &[];
            pub const WRAPPER_COMMANDS: &[&str] = &["npx"];
            pub const GIT_SUBCOMMANDS: &[&str] = &[];
            pub const TRANSPARENT_RUNNERS: &[&str] = &["npx"];
            """
        )
        rows = usage_report.normalize_rows(
            [
                ("codex", "mystery-tool --help"),
                ("codex", "npx mystery-tool --help"),
            ]
        )

        report = usage_report.build_report(rows, catalog)

        self.assertEqual(
            report["unlisted_effective_commands"],
            [
                {
                    "command": "mystery-tool",
                    "count": 2,
                    "routing_coverage": "unlisted",
                    "compaction_coverage": "not-catalogued",
                    "runtime_dispatchable_count": 0,
                    "runner_chains": [{"chain": ["npx"], "count": 1}],
                }
            ],
        )
        self.assertEqual(
            report["runner_chains"],
            [{"chain": ["npx"], "count": 1, "runtime_dispatchable_count": 0}],
        )

    def test_pnpm_option_led_direct_invocation_is_not_treated_as_exec(self) -> None:
        catalog = usage_report.parse_catalog(
            """
            pub const AUTO_WRAP_COMMANDS: &[&str] = &["pnpm"];
            pub const WRAPPER_COMMANDS: &[&str] = &["pnpm"];
            pub const GIT_SUBCOMMANDS: &[&str] = &[];
            pub const TRANSPARENT_RUNNERS: &[&str] = &["pnpm exec"];
            """
        )
        rows = usage_report.normalize_rows(
            [
                ("codex", "pnpm --recursive test"),
                ("codex", "pnpm --filter exec test"),
                ("codex", "pnpm --recursive -- exec pytest"),
                ("codex", "pnpm --recursive exec pytest"),
                ("codex", "pnpm --future exec pytest"),
            ]
        )

        report = usage_report.build_report(rows, catalog)
        effective = {record["command"]: record for record in report["effective_commands"]}

        self.assertEqual(effective["pnpm"]["routing_coverage"], "auto-wrap")
        self.assertEqual(effective["pnpm"]["runtime_dispatchable_count"], 3)
        self.assertEqual(effective["pytest"]["runtime_dispatchable_count"], 0)
        self.assertEqual(
            report["runner_chains"],
            [{"chain": ["pnpm exec"], "count": 2, "runtime_dispatchable_count": 0}],
        )


if __name__ == "__main__":
    unittest.main()
