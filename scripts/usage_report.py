#!/usr/bin/env python3
"""Report command usage from coding-agent session history.

The report is intentionally read-only. It accepts opencode's SQLite database
and JSONL session roots used by Codex or Claude, then compares observed
commands with the tapas-owned catalog.
"""

from __future__ import annotations

import argparse
import collections
import json
import pathlib
import re
import shlex
import sqlite3
import sys
from typing import Any, Iterable, Iterator


ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_OPENCODE_DB = pathlib.Path.home() / ".local/share/opencode/opencode.db"
DEFAULT_CODEX_ROOT = pathlib.Path.home() / ".codex/sessions"
DEFAULT_CLAUDE_ROOT = pathlib.Path.home() / ".claude/projects"
COMMAND_KEYS = {"command", "cmd", "shell_command", "command_line"}
OPERATORS = {"&&", "||", ";", "|", "&"}
SHELLS = {"bash", "sh", "zsh", "fish"}
SHELL_BUILTINS = {"cd", "export", "popd", "pushd", "set", "source", "unset"}
SHELL_KEYWORDS = {
    "case",
    "do",
    "done",
    "elif",
    "else",
    "esac",
    "fi",
    "for",
    "function",
    "if",
    "in",
    "then",
    "while",
}
WRAPPERS = {"smll", "tapas"}


def parse_catalog(source: str) -> dict[str, set[str]]:
    """Parse the static command catalog without importing Rust code."""

    result: dict[str, set[str]] = {}
    for name in (
        "AUTO_WRAP_COMMANDS",
        "WRAPPER_COMMANDS",
        "GIT_SUBCOMMANDS",
        "TRANSPARENT_RUNNERS",
    ):
        match = re.search(
            rf"pub const {name}:\s*&\s*\[\s*&\s*str\]\s*=\s*&\[(.*?)\];",
            source,
            re.S,
        )
        if not match:
            raise ValueError(f"catalog constant not found: {name}")
        result[name] = set(re.findall(r'"([^"\\]+)"', match.group(1)))
    return result


def basename(value: str) -> str:
    return pathlib.PurePosixPath(value.replace("\\", "/")).name


def _is_assignment(word: str) -> bool:
    return bool(re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", word))


def normalize_invocation(command: str) -> tuple[str, list[str]] | None:
    """Return the effective executable and argv after common wrappers."""

    try:
        words = shlex.split(command, posix=True)
    except ValueError:
        return None
    if not words:
        return None

    index = 0
    while index < len(words):
        word = words[index]
        if _is_assignment(word) or word in OPERATORS:
            index += 1
            continue
        name = basename(word)
        if name in SHELL_BUILTINS:
            index += 1
            while index < len(words) and words[index] not in OPERATORS:
                index += 1
            continue
        if name == "env":
            index += 1
            while index < len(words) and (_is_assignment(words[index]) or words[index].startswith("-")):
                index += 1
            continue
        if name == "command" or name == "exec":
            index += 1
            continue
        if name in SHELLS:
            shell_argument = next(
                (position + 1 for position, value in enumerate(words[index + 1 :], index + 1) if value in {"-c", "-lc", "-cl"}),
                None,
            )
            if shell_argument is None or shell_argument >= len(words):
                return None
            return normalize_invocation(words[shell_argument])
        if name in SHELL_KEYWORDS:
            return None
        if name in WRAPPERS:
            index += 1
            while index < len(words) and words[index].startswith("-"):
                if words[index] == "--":
                    index += 1
                    break
                index += 1
            continue
        if name.startswith("#"):
            return None
        return name, words[index + 1 :]
    return None


def _embedded_commands(value: str) -> Iterator[str]:
    """Extract commands embedded in tool-call argument strings."""

    try:
        parsed = json.loads(value)
    except (TypeError, json.JSONDecodeError):
        parsed = None
    if parsed is not None:
        yield from _commands_from_object(parsed)
        return
    for match in re.finditer(r"(?:['\"](?:command|cmd)['\"]\s*[:=]\s*)(['\"])(.*?)\1", value):
        yield match.group(2)


def _commands_from_object(value: Any) -> Iterator[str]:
    if isinstance(value, dict):
        for key, item in value.items():
            if key in COMMAND_KEYS and isinstance(item, str):
                yield item
            elif key in {"arguments", "input", "tool_input"} and isinstance(item, str):
                yield from _embedded_commands(item)
            else:
                yield from _commands_from_object(item)
    elif isinstance(value, list):
        for item in value:
            yield from _commands_from_object(item)


def commands_from_json_line(line: str) -> Iterator[str]:
    try:
        record = json.loads(line)
    except json.JSONDecodeError:
        return
    found = set(_commands_from_object(record))
    # Codex custom tool calls sometimes contain JavaScript source in `input`,
    # where the command is encoded as an object literal rather than JSON.
    for match in re.finditer(r"['\"](?:command|cmd)['\"]\s*:\s*(['\"])(.*?)\1", line):
        found.add(match.group(2).replace('\\"', '"').replace('\\n', '\n'))
    yield from sorted(found)


def collect_opencode(path: pathlib.Path) -> list[tuple[str, str]]:
    if not path.is_file():
        return []
    rows: list[tuple[str, str]] = []
    try:
        with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as connection:
            query = """
                SELECT data
                FROM part
                WHERE json_valid(data)
                  AND json_extract(data, '$.type') = 'tool'
                  AND json_extract(data, '$.tool') = 'bash'
            """
            for (data,) in connection.execute(query):
                try:
                    record = json.loads(data)
                except (TypeError, json.JSONDecodeError):
                    continue
                command = record.get("state", {}).get("input", {}).get("command")
                if isinstance(command, str):
                    rows.append(("opencode", command))
    except sqlite3.Error:
        return rows
    return rows


def collect_jsonl(root: pathlib.Path, source: str) -> list[tuple[str, str]]:
    if not root.is_dir():
        return []
    rows: list[tuple[str, str]] = []
    for path in sorted(root.rglob("*.jsonl")):
        try:
            with path.open(encoding="utf-8", errors="replace") as handle:
                for line in handle:
                    rows.extend((source, command) for command in commands_from_json_line(line))
        except OSError:
            continue
    return rows


def normalize_rows(rows: Iterable[tuple[str, str]]) -> list[dict[str, Any]]:
    normalized: list[dict[str, Any]] = []
    for source, raw in rows:
        invocation = normalize_invocation(raw)
        if invocation is None:
            continue
        command, arguments = invocation
        normalized.append(
            {
                "source": source,
                "command": command,
                "arguments": arguments,
                "raw": raw,
            }
        )
    return normalized


def first_subcommand(arguments: list[str]) -> str | None:
    skip_next = {"-C", "--git-dir", "--work-tree", "--repo", "--hostname"}
    index = 0
    while index < len(arguments):
        argument = arguments[index]
        if argument in skip_next:
            index += 2
            continue
        if argument == "-c":
            index += 2
            continue
        if argument.startswith("-c") and len(argument) > 2:
            index += 1
            continue
        if argument.startswith("-"):
            index += 1
            continue
        return argument
    return None


def build_report(rows: Iterable[dict[str, Any]], catalog: dict[str, set[str]], minimum: int = 1) -> dict[str, Any]:
    rows = list(rows)
    commands = collections.Counter(row["command"] for row in rows)
    sources = collections.Counter(row["source"] for row in rows)
    command_records = []
    transparent_prefixes = [runner.split() for runner in catalog["TRANSPARENT_RUNNERS"]]

    def coverage_for(row: dict[str, Any]) -> str:
        command = row["command"]
        if command in catalog["AUTO_WRAP_COMMANDS"]:
            return "auto-wrap"
        if any(
            [command, *row["arguments"]][: len(prefix)] == prefix
            for prefix in transparent_prefixes
        ):
            return "transparent-runner"
        if command in catalog["WRAPPER_COMMANDS"]:
            return "wrapper-only"
        return "unlisted"

    for command, count in commands.most_common():
        if count < minimum:
            continue
        statuses = {
            coverage_for(row) for row in rows if row["command"] == command
        }
        coverage = next(iter(statuses)) if len(statuses) == 1 else "mixed"
        command_records.append({"command": command, "count": count, "coverage": coverage})

    git_subcommands = collections.Counter(
        first_subcommand(row["arguments"])
        for row in rows
        if row["command"] == "git" and first_subcommand(row["arguments"])
    )
    git_records = [
        {
            "subcommand": subcommand,
            "count": count,
            "coverage": "catalogued" if subcommand in catalog["GIT_SUBCOMMANDS"] else "unlisted",
        }
        for subcommand, count in git_subcommands.most_common()
        if count >= minimum
    ]
    return {
        "total_invocations": len(rows),
        "sources": dict(sources),
        "commands": command_records,
        "git_subcommands": git_records,
        "unlisted_commands": [record for record in command_records if record["coverage"] == "unlisted"],
        "unlisted_git_subcommands": [record for record in git_records if record["coverage"] == "unlisted"],
    }


def format_text(report: dict[str, Any]) -> str:
    lines = [f"Total invocations: {report['total_invocations']}", "Sources:"]
    lines.extend(f"  {source}: {count}" for source, count in sorted(report["sources"].items()))
    lines.append("Commands:")
    lines.extend(
        f"  {record['command']}: {record['count']} ({record['coverage']})"
        for record in report["commands"]
    )
    lines.append("Git subcommands:")
    lines.extend(
        f"  {record['subcommand']}: {record['count']} ({record['coverage']})"
        for record in report["git_subcommands"]
    )
    return "\n".join(lines) + "\n"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--opencode-db", type=pathlib.Path, default=DEFAULT_OPENCODE_DB)
    parser.add_argument("--codex-root", type=pathlib.Path, default=DEFAULT_CODEX_ROOT)
    parser.add_argument("--claude-root", type=pathlib.Path, default=DEFAULT_CLAUDE_ROOT)
    parser.add_argument("--catalog", type=pathlib.Path, default=ROOT / "src/catalog.rs")
    parser.add_argument("--minimum", type=int, default=1)
    parser.add_argument("--format", choices=("text", "json"), default="text")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.minimum < 1:
        print("--minimum must be at least 1", file=sys.stderr)
        return 2
    try:
        catalog = parse_catalog(args.catalog.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        print(f"could not read catalog: {exc}", file=sys.stderr)
        return 2
    raw_rows = []
    raw_rows.extend(collect_opencode(args.opencode_db))
    raw_rows.extend(collect_jsonl(args.codex_root, "codex"))
    raw_rows.extend(collect_jsonl(args.claude_root, "claude"))
    report = build_report(normalize_rows(raw_rows), catalog, args.minimum)
    if args.format == "json":
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(format_text(report), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
