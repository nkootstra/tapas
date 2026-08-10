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

from catalog_source import parse_catalog


ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_OPENCODE_DB = pathlib.Path.home() / ".local/share/opencode/opencode.db"
DEFAULT_CODEX_ROOT = pathlib.Path.home() / ".codex/sessions"
DEFAULT_CLAUDE_ROOT = pathlib.Path.home() / ".claude/projects"
COMMAND_KEYS = {"command", "cmd", "shell_command", "command_line"}
SHELL_TOOL_NAMES = {"bash", "exec_command", "shell", "shell_command"}
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
UV_RUNNER_VALUE_OPTIONS = {
    "--project", "--directory", "--python", "--package", "--with",
    "--with-editable", "--with-requirements", "--env-file", "--group", "--extra",
}
UVX_VALUE_OPTIONS = {*UV_RUNNER_VALUE_OPTIONS, "--from"}
UV_BOOLEAN_OPTIONS = {
    "--isolated", "--active", "--no-sync", "--locked", "--frozen",
    "--no-project", "--all-extras", "--no-dev", "--no-progress", "--offline",
}
MAX_RUNNER_LAYERS = 4


def basename(value: str) -> str:
    return pathlib.PurePosixPath(value.replace("\\", "/")).name


def _is_shell_tool(value: Any) -> bool:
    return isinstance(value, str) and basename(value).rsplit(".", 1)[-1].lower() in SHELL_TOOL_NAMES


def _is_assignment(word: str) -> bool:
    return bool(re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", word))


def _split_shell_commands(command: str) -> list[str] | None:
    """Split shell command chains without splitting quoted command text."""

    segments: list[str] = []
    start = 0
    quote: str | None = None
    escaped = False
    index = 0
    while index < len(command):
        character = command[index]
        if escaped:
            escaped = False
            index += 1
            continue
        if quote == "'":
            if character == "'":
                quote = None
            index += 1
            continue
        if quote is not None:
            if character == "\\":
                escaped = True
            elif character == quote:
                quote = None
            index += 1
            continue
        if character == "\\":
            escaped = True
            index += 1
            continue
        if character in {"'", '"'}:
            quote = character
            index += 1
            continue

        operator = next(
            (
                candidate
                for candidate in ("&&", "||", "|&", ";", "|", "&", "\n")
                if command.startswith(candidate, index)
            ),
            None,
        )
        if operator is not None:
            # `&` in a redirection such as `2>&1` or `&>file` is not a
            # command separator.
            if operator == "&" and (
                command[index - 1 : index] in {"<", ">"}
                or command[index + 1 : index + 2] == ">"
            ):
                index += 1
                continue
            segments.append(command[start:index])
            index += len(operator)
            start = index
            continue
        index += 1

    if quote is not None or escaped:
        return None
    segments.append(command[start:])
    return segments


def _normalize_words(words: list[str]) -> list[tuple[str, list[str]]] | None:
    if not words:
        return []
    index = 0
    normalized: list[tuple[str, list[str]]] = []
    while index < len(words):
        word = words[index]
        if _is_assignment(word):
            index += 1
            continue
        name = basename(word)
        if name in SHELL_BUILTINS:
            index += 1
            index = len(words)
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
            nested = normalize_invocation(words[shell_argument])
            return None if nested is None else [*normalized, *nested]
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
        normalized.append((name, words[index + 1 :]))
        return normalized
    return normalized


def normalize_invocation(command: str) -> list[tuple[str, list[str]]] | None:
    """Return effective executables and argv after common wrappers."""

    segments = _split_shell_commands(command)
    if segments is None:
        return None

    normalized: list[tuple[str, list[str]]] = []
    for segment in segments:
        try:
            words = shlex.split(segment, posix=True)
        except ValueError:
            return None
        segment_invocations = _normalize_words(words)
        if segment_invocations is None:
            return None
        normalized.extend(segment_invocations)
    return normalized or None


def _commands_from_arguments(value: Any) -> Iterator[str]:
    """Extract command fields from the arguments of a known tool call."""

    if isinstance(value, dict):
        for key in COMMAND_KEYS:
            command = value.get(key)
            if isinstance(command, str):
                yield command


def _embedded_commands(value: str, *, javascript: bool = False) -> Iterator[str]:
    """Extract commands embedded in the arguments of a known tool call."""

    try:
        parsed = json.loads(value)
    except (TypeError, json.JSONDecodeError):
        parsed = None
    if parsed is not None:
        yield from _commands_from_arguments(parsed)
        return
    if javascript:
        for match in re.finditer(
            r"tools\.exec_command\s*\(\s*\{[^{}]*?['\"]?(?:command|cmd)['\"]?\s*:\s*(['\"])(.*?)\1",
            value,
            re.DOTALL,
        ):
            yield match.group(2).replace(r'\"', '"').replace(r"\n", "\n")


def _commands_from_object(value: Any) -> Iterator[str]:
    if isinstance(value, dict):
        record_type = value.get("type")
        if record_type == "function_call":
            if not _is_shell_tool(value.get("name")):
                return
            arguments = value.get("arguments")
            if isinstance(arguments, str):
                yield from _embedded_commands(arguments)
            elif isinstance(arguments, dict):
                yield from _commands_from_arguments(arguments)
            return
        if record_type == "custom_tool_call":
            tool_name = value.get("name")
            tool_input = value.get("input")
            if tool_name in {"functions.exec", "exec"} and isinstance(tool_input, str):
                yield from _embedded_commands(tool_input, javascript=True)
            return
        if record_type == "tool_use":
            tool_name = value.get("name")
            tool_input = value.get("input")
            if _is_shell_tool(tool_name):
                yield from _commands_from_arguments(tool_input)
            return
        for item in value.values():
            if isinstance(item, (dict, list)):
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
        invocations = normalize_invocation(raw)
        if invocations is None:
            continue
        normalized.extend(
            {
                "source": source,
                "command": command,
                "arguments": arguments,
                "raw": raw,
            }
            for command, arguments in invocations
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


def _option_kind(argument: str, options: set[str]) -> str | None:
    if argument in options:
        return "separate"
    for option in options:
        if len(option) > 2 and argument.startswith(option + "="):
            return "inline"
        if len(option) == 2 and argument.startswith(option) and len(argument) > 2:
            return "inline"
    return None


def _matches_boolean_option(argument: str, options: set[str]) -> bool:
    if argument in options:
        return True
    return (
        len(argument) > 2
        and argument.startswith("-")
        and not argument.startswith("--")
        and all(f"-{flag}" in options for flag in argument[1:])
    )


def _scan_runner_options(
    arguments: list[str],
    start: int,
    *,
    values: set[str],
    booleans: set[str],
    opaque: set[str] = frozenset(),
    stop: str | None = None,
) -> int | None:
    index = start
    while index < len(arguments):
        argument = arguments[index]
        if argument == stop:
            return index
        if argument == "--":
            return index + 1 if stop is None else None
        if not argument.startswith("-"):
            return index if stop is None else None
        if _option_kind(argument, opaque) is not None:
            return None
        if _matches_boolean_option(argument, booleans):
            index += 1
            continue
        value_kind = _option_kind(argument, values)
        if value_kind == "inline":
            index += 1
        elif value_kind == "separate" and index + 1 < len(arguments):
            index += 2
        else:
            return None
    return None


def _runner_step(
    command: str, arguments: list[str], declared: set[str]
) -> tuple[str, tuple[str, list[str]] | None] | None:
    runner: str | None = None
    index: int | None = None
    if command == "uv" and "uv run" in declared and arguments[:1] == ["run"]:
        runner = "uv run"
        index = _scan_runner_options(
            arguments, 1, values=UV_RUNNER_VALUE_OPTIONS, booleans=UV_BOOLEAN_OPTIONS
        )
    elif command == "uvx" and "uvx" in declared:
        runner = "uvx"
        index = _scan_runner_options(
            arguments, 0, values=UVX_VALUE_OPTIONS, booleans=UV_BOOLEAN_OPTIONS
        )
    elif command == "poetry" and "poetry run" in declared:
        runner = "poetry run"
        index = _scan_runner_options(
            arguments,
            0,
            values={"-C", "--directory", "-P", "--project"},
            booleans={"--no-interaction", "--no-ansi", "-q", "--quiet"},
            stop="run",
        )
        index = None if index is None else index + 1
    elif command == "pnpm" and "pnpm exec" in declared and (
        "exec" in arguments or (arguments and arguments[0].startswith("-"))
    ):
        runner = "pnpm exec"
        index = _scan_runner_options(
            arguments,
            0,
            values={"-C", "--dir", "-F", "--filter", "--workspace-concurrency"},
            booleans={
                "-r", "--recursive", "-w", "--workspace-root", "--parallel",
                "--stream", "--aggregate-output", "--use-stderr",
            },
            stop="exec",
        )
        index = None if index is None else index + 1
    elif command == "npx" and "npx" in declared:
        runner = "npx"
        index = _scan_runner_options(
            arguments,
            0,
            values={"-p", "--package", "-w", "--workspace", "--allow-scripts"},
            booleans={
                "-y", "--yes", "--no", "--workspaces", "--include-workspace-root",
                "--strict-allow-scripts", "--dangerously-allow-all-scripts",
            },
            opaque={"-c", "--call"},
        )
    elif command == "bunx" and "bunx" in declared:
        runner = "bunx"
        index = _scan_runner_options(
            arguments,
            0,
            values={"--package", "--cwd"},
            booleans={"--bun", "--no-install", "--silent", "--help", "--version"},
        )
    if runner is None:
        return None
    if index is None or index >= len(arguments) or arguments[index].startswith("-"):
        return runner, None
    return runner, (basename(arguments[index]), arguments[index + 1 :])


def _effective_invocation(
    row: dict[str, Any], transparent_prefixes: list[list[str]]
) -> dict[str, Any]:
    original_command = row["command"]
    original_arguments = row["arguments"]
    declared = {" ".join(prefix) for prefix in transparent_prefixes}
    command = original_command
    arguments = original_arguments
    runner_chain: list[str] = []

    for _ in range(MAX_RUNNER_LAYERS):
        runner = _runner_step(command, arguments, declared)
        if runner is None:
            return {
                "command": command,
                "arguments": arguments,
                "runner_chain": runner_chain,
                "runtime_dispatchable": True,
                "runner_credited": bool(runner_chain),
            }
        runner_name, logical = runner
        runner_chain.append(runner_name)
        if logical is None:
            return {
                "command": original_command,
                "arguments": original_arguments,
                "runner_chain": runner_chain,
                "runtime_dispatchable": False,
                "runner_credited": False,
            }
        command, arguments = logical

    nested = _runner_step(command, arguments, declared)
    if nested is None:
        return {
            "command": command,
            "arguments": arguments,
            "runner_chain": runner_chain,
            "runtime_dispatchable": True,
            "runner_credited": True,
        }

    runner_chain.append(nested[0])
    return {
        "command": original_command,
        "arguments": original_arguments,
        "runner_chain": runner_chain,
        "runtime_dispatchable": False,
        "runner_credited": False,
    }


def build_report(rows: Iterable[dict[str, Any]], catalog: dict[str, set[str]], minimum: int = 1) -> dict[str, Any]:
    rows = list(rows)
    commands = collections.Counter(row["command"] for row in rows)
    rows_by_command: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
    for row in rows:
        rows_by_command[row["command"]].append(row)
    sources = collections.Counter(row["source"] for row in rows)
    command_records = []
    transparent_prefixes = [runner.split() for runner in catalog["TRANSPARENT_RUNNERS"]]
    compact_commands = {
        command
        for route in catalog.get("COMPACT_ROUTES", set())
        for command in route.split(":", 1)[0].split("/")
    }

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

    def direct_coverage(command: str) -> str:
        if command in catalog["AUTO_WRAP_COMMANDS"]:
            return "auto-wrap"
        if command in catalog["WRAPPER_COMMANDS"]:
            return "wrapper-only"
        return "unlisted"

    for command, count in commands.most_common():
        if count < minimum:
            continue
        statuses = {coverage_for(row) for row in rows_by_command[command]}
        coverage = next(iter(statuses)) if len(statuses) == 1 else "mixed"
        command_records.append({"command": command, "count": count, "coverage": coverage})

    git_subcommands: collections.Counter[str] = collections.Counter()
    for row in rows_by_command.get("git", []):
        if subcommand := first_subcommand(row["arguments"]):
            git_subcommands[subcommand] += 1
    git_records = [
        {
            "subcommand": subcommand,
            "count": count,
            "coverage": "catalogued" if subcommand in catalog["GIT_SUBCOMMANDS"] else "unlisted",
        }
        for subcommand, count in git_subcommands.most_common()
        if count >= minimum
    ]
    effective_rows = []
    effective_by_command: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
    for row in rows:
        effective = {**row, **_effective_invocation(row, transparent_prefixes)}
        effective_rows.append(effective)
        effective_by_command[effective["command"]].append(effective)
    effective_counts = collections.Counter(row["command"] for row in effective_rows)
    effective_records = []
    for command, count in effective_counts.most_common():
        if count < minimum:
            continue
        matching = effective_by_command[command]
        routing_statuses = {
            "transparent-runner"
            if row["runner_credited"]
            else direct_coverage(row["command"])
            for row in matching
        }
        routing_coverage = (
            next(iter(routing_statuses)) if len(routing_statuses) == 1 else "mixed"
        )
        compaction_statuses = {
            "catalogued-route"
            if command in compact_commands
            else "not-catalogued"
            if status == "unlisted"
            else "route-undeclared"
            for status in routing_statuses
        }
        compaction_coverage = (
            next(iter(compaction_statuses))
            if len(compaction_statuses) == 1
            else "mixed"
        )
        chains = collections.Counter(
            tuple(row["runner_chain"]) for row in matching if row["runner_chain"]
        )
        effective_records.append(
            {
                "command": command,
                "count": count,
                "routing_coverage": routing_coverage,
                "compaction_coverage": compaction_coverage,
                "runtime_dispatchable_count": sum(
                    row["runtime_dispatchable"] for row in matching
                ),
                "runner_chains": [
                    {"chain": list(chain), "count": chain_count}
                    for chain, chain_count in sorted(chains.items())
                ],
            }
        )
    chain_counts: collections.Counter[tuple[str, ...]] = collections.Counter()
    chain_dispatchable: collections.Counter[tuple[str, ...]] = collections.Counter()
    for row in effective_rows:
        if row["runner_chain"]:
            chain = tuple(row["runner_chain"])
            chain_counts[chain] += 1
            chain_dispatchable[chain] += row["runtime_dispatchable"]
    runner_chain_records = [
        {
            "chain": list(chain),
            "count": count,
            "runtime_dispatchable_count": chain_dispatchable[chain],
        }
        for chain, count in sorted(chain_counts.items())
        if count >= minimum
    ]
    return {
        "total_invocations": len(rows),
        "sources": dict(sources),
        "commands": command_records,
        "git_subcommands": git_records,
        "unlisted_commands": [record for record in command_records if record["coverage"] == "unlisted"],
        "unlisted_git_subcommands": [record for record in git_records if record["coverage"] == "unlisted"],
        "effective_commands": effective_records,
        "unlisted_effective_commands": [
            record
            for record in effective_records
            if record["routing_coverage"] == "unlisted"
        ],
        "runner_chains": runner_chain_records,
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
    lines.append("Effective commands:")
    lines.extend(
        f"  {record['command']}: {record['count']} "
        f"(routing={record['routing_coverage']}, "
        f"compaction={record['compaction_coverage']}, "
        f"runtime-dispatchable={record['runtime_dispatchable_count']})"
        for record in report["effective_commands"]
    )
    lines.append("Runner chains:")
    lines.extend(
        f"  {' -> '.join(record['chain'])}: {record['count']} "
        f"(runtime-dispatchable={record['runtime_dispatchable_count']})"
        for record in report["runner_chains"]
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
