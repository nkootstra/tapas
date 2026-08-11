#!/usr/bin/env python3
"""Report command usage from coding-agent session history.

The report is intentionally read-only. It accepts opencode's SQLite database
and JSONL session roots used by Codex or Claude, then compares observed
commands with the tapas-owned catalog.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import secrets
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
DEFAULT_EXCLUDE_COMMAND_FILE = pathlib.Path.home() / ".config" / "tapas" / "usage-report-excluded-commands"
COMMAND_KEYS = {"command", "cmd", "shell_command", "command_line"}
SHELLS = {"bash", "sh", "zsh", "fish"}
ESTIMATED_BYTES_PER_TOKEN = 4
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
COMMAND_NOISE_WORDS = {"[", "]", "{", "}", "(", ")", "EOF"}
_REDACT_SALT = secrets.token_hex(16)


def basename(value: str) -> str:
    return pathlib.PurePosixPath(value.replace("\\", "/")).name


def _is_assignment(word: str) -> bool:
    return bool(re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", word))


def _is_command_noise(word: str) -> bool:
    if word in COMMAND_NOISE_WORDS:
        return True
    if word.startswith("-") and len(word) > 1:
        return True
    if len(word) <= 2 and all(
        character in {"+", "-", "{", "}", "[", "]", "(", ")"} for character in word
    ):
        return True
    return False


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
        if _is_command_noise(name):
            index += 1
            continue
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


def _metric_int(value: Any) -> int:
    try:
        return max(int(value), 0)
    except (TypeError, ValueError):
        return 0


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


def load_excluded_commands(path: pathlib.Path) -> set[str]:
    if not path.is_file():
        return set()
    excluded: set[str] = set()
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            command = line.strip()
            if not command or command.startswith("#"):
                continue
            excluded.add(command.lower())
    except OSError:
        return set()
    return excluded


def load_compaction_metrics(path: pathlib.Path | None) -> dict[str, dict[str, Any]]:
    if path is None or not path.is_file():
        return {}
    metrics: dict[str, dict[str, Any]] = {}

    try:
        with path.open(encoding="utf-8", errors="replace") as handle:
            for line in handle:
                try:
                    raw = json.loads(line)
                except json.JSONDecodeError:
                    continue
                command = raw.get("command")
                if not isinstance(command, str) or not command:
                    continue
                entry = metrics.setdefault(
                    command,
                    {
                        "invocations": 0,
                        "raw_bytes": 0,
                        "displayed_bytes": 0,
                        "diagnostic_bytes": 0,
                        "saved_invocations": 0,
                        "saved_bytes": 0,
                    },
                )
                entry["invocations"] += 1
                raw_bytes = _metric_int(raw.get("raw_bytes", 0))
                displayed_bytes = _metric_int(raw.get("displayed_bytes", 0))
                diagnostic_bytes = _metric_int(raw.get("diagnostic_bytes", 0))
                entry["raw_bytes"] += raw_bytes
                entry["displayed_bytes"] += displayed_bytes
                entry["diagnostic_bytes"] += diagnostic_bytes
                if raw.get("changed", False):
                    entry["saved_invocations"] += 1
                    entry["saved_bytes"] += max(raw_bytes - displayed_bytes, 0)
    except OSError:
        return {}
    return metrics


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


def build_report(
    rows: Iterable[dict[str, Any]],
    catalog: dict[str, set[str]],
    minimum: int = 1,
    *,
    compaction_metrics: dict[str, dict[str, Any]] | None = None,
    readable: bool = False,
    excluded_commands: set[str] | frozenset[str] = frozenset(),
    include_noise: bool = False,
) -> dict[str, Any]:
    rows = list(rows)
    commands = collections.Counter(row["command"] for row in rows)
    sources = collections.Counter(row["source"] for row in rows)
    compaction_metrics = compaction_metrics or {}
    command_sources = collections.defaultdict(collections.Counter)
    for row in rows:
        command_sources[row["command"]][row["source"]] += 1
    command_records = []
    transparent_prefixes = [runner.split() for runner in catalog["TRANSPARENT_RUNNERS"]]
    excluded_commands = {value.lower() for value in excluded_commands}

    def coverage_for(command: str, arguments: list[str]) -> str:
        if command in catalog["AUTO_WRAP_COMMANDS"]:
            return "auto-wrap"
        if any(
            [command, *arguments][: len(prefix)] == prefix
            for prefix in transparent_prefixes
        ):
            return "transparent-runner"
        if command in catalog["WRAPPER_COMMANDS"]:
            return "wrapper-only"
        return "unlisted"

    coverage_invocations = collections.Counter()
    for row in rows:
        coverage_invocations[coverage_for(row["command"], row["arguments"])] += 1
    status_cache = {}
    for command, count in commands.items():
        if count < minimum:
            continue
        metric = compaction_metrics.get(command, {})
        status_cache.setdefault(
            command,
            {
                coverage_for(
                    row["command"], row["arguments"]
                )
                for row in rows
                if row["command"] == command
            },
        )
        statuses = status_cache[command]
        coverage = next(iter(statuses)) if len(statuses) == 1 else "mixed"
        if readable and not include_noise and command.lower() not in excluded_commands:
            continue
        record = {
            "command": command,
            "count": count,
            "coverage": coverage,
        }
        if metric:
            record["estimated_saved_bytes"] = metric["saved_bytes"]
            record["estimated_saved_invocations"] = metric["saved_invocations"]
            record["estimated_saved_tokens"] = metric["saved_bytes"] // ESTIMATED_BYTES_PER_TOKEN
        if readable:
            record["sources"] = dict(sorted(command_sources[command].items()))
        command_records.append(record)

    command_records.sort(key=lambda item: (-item["count"], item["command"]))

    git_subcommands = collections.Counter(
        first_subcommand(row["arguments"])
        for row in rows
        if row["command"] == "git" and first_subcommand(row["arguments"])
    )
    git_subcommand_sources = collections.defaultdict(collections.Counter)
    if readable:
        for row in rows:
            if row["command"] != "git":
                continue
            subcommand = first_subcommand(row["arguments"])
            if subcommand:
                git_subcommand_sources[subcommand][row["source"]] += 1
    git_records = [
        {
            "subcommand": subcommand,
            "count": count,
            "coverage": "catalogued" if subcommand in catalog["GIT_SUBCOMMANDS"] else "unlisted",
            **(
                {"sources": dict(sorted(git_subcommand_sources[subcommand].items()))}
                if readable
                else {}
            ),
        }
        for subcommand, count in git_subcommands.most_common()
        if count >= minimum
    ]
    if readable:
        git_records.sort(key=lambda item: (-item["count"], item["subcommand"]))

    compaction_candidates = [
        {
            "command": record["command"],
            "count": record["count"],
            "estimated_saved_bytes": record.get("estimated_saved_bytes", 0),
            "estimated_saved_invocations": record.get("estimated_saved_invocations", 0),
            "estimated_saved_tokens": record.get("estimated_saved_tokens", 0),
            "coverage": record["coverage"],
        }
        for record in command_records
        if record["coverage"] == "unlisted"
        and (include_noise or record["command"].lower() not in excluded_commands)
        and record.get("estimated_saved_bytes", 0) > 0
    ]
    compaction_candidates.sort(
        key=lambda item: (
            -item["estimated_saved_bytes"],
            -item["estimated_saved_invocations"],
            item["command"],
        )
    )
    total_invocations = len(rows)
    covered_invocations = (
        total_invocations
        - coverage_invocations.get("unlisted", 0)
    )
    return {
        "total_invocations": total_invocations,
        "sources": dict(sources),
        "commands": command_records,
        "git_subcommands": git_records,
        "coverage_invocations": dict(coverage_invocations),
        "coverage_summary": {
            "covered_invocations": covered_invocations,
            "unlisted_invocations": coverage_invocations.get("unlisted", 0),
        },
        "compaction_candidates": compaction_candidates,
        "unlisted_commands": [record for record in command_records if record["coverage"] == "unlisted"],
        "unlisted_git_subcommands": [record for record in git_records if record["coverage"] == "unlisted"],
    }


def format_text(report: dict[str, Any]) -> str:
    return _format_text_with_redaction(report, redact=False)


def _redact_identifier(
    value: str, *, mapping: dict[str, str], prefix: str
) -> str:
    if value in mapping:
        return mapping[value]
    token = f"{prefix}-{hashlib.sha1(f'{_REDACT_SALT}:{value}'.encode('utf-8')).hexdigest()[:10]}"
    mapping[value] = token
    return token


def _format_text_with_redaction(
    report: dict[str, Any],
    *,
    redact: bool = False,
    command_aliases: dict[str, str] | None = None,
    git_aliases: dict[str, str] | None = None,
) -> str:
    command_aliases = command_aliases or {}
    git_aliases = git_aliases or {}
    lines = [f"Total invocations: {report['total_invocations']}", "Sources:"]
    lines.extend(f"  {source}: {count}" for source, count in sorted(report["sources"].items()))
    lines.append("Commands:")
    lines.extend(
        f"  { _redact_identifier(record['command'], mapping=command_aliases, prefix='cmd') if redact else record['command']}: {record['count']} ({record['coverage']})"
        for record in report["commands"]
    )
    lines.append("Git subcommands:")
    lines.extend(
        f"  {_redact_identifier(record['subcommand'], mapping=git_aliases, prefix='git') if redact else record['subcommand']}: {record['count']} ({record['coverage']})"
        for record in report["git_subcommands"]
    )
    return "\n".join(lines) + "\n"


def _format_sources(sources: dict[str, int] | None) -> str:
    if not sources:
        return "source unknown"
    return ", ".join(f"{source}={count}" for source, count in sorted(sources.items()))


def format_compaction_plan(
    report: dict[str, Any],
    *,
    top_n: int,
    include_noise: bool,
    excluded_commands: set[str] | frozenset[str],
    redact: bool = False,
    command_aliases: dict[str, str] | None = None,
    git_aliases: dict[str, str] | None = None,
) -> str:
    excluded_commands = {value.lower() for value in excluded_commands}
    command_aliases = command_aliases or {}
    git_aliases = git_aliases or {}
    total_invocations = report["total_invocations"]
    covered_invocations = report["coverage_summary"]["covered_invocations"]
    unlisted_invocations = report["coverage_summary"]["unlisted_invocations"]
    coverage_ratio = (
        (covered_invocations / total_invocations * 100) if total_invocations else 0.0
    )
    lines = [
        f"Total invocations: {total_invocations}",
        f"Command-level compaction: {covered_invocations}/{total_invocations} covered ({coverage_ratio:.1f}%)",
        f"Unlisted invocations: {unlisted_invocations}",
        "Invocation coverage breakdown:",
    ]
    lines.extend(
        f"  {status}: {count}"
        for status, count in sorted(report["coverage_invocations"].items())
    )

    compaction_candidates = list(report.get("compaction_candidates", []))
    if not compaction_candidates:
        compaction_candidates = sorted(
            [
                {
                    "command": record["command"],
                    "count": record["count"],
                    "estimated_saved_bytes": 0,
                    "estimated_saved_invocations": 0,
                    "estimated_saved_tokens": 0,
                    "coverage": record["coverage"],
                }
                for record in report["unlisted_commands"]
                if include_noise or record["command"].lower() not in excluded_commands
            ],
            key=lambda item: (-item["count"], item["command"]),
        )
    else:
        compaction_candidates.sort(
            key=lambda item: (
                -item["estimated_saved_bytes"],
                -item["estimated_saved_invocations"],
                -item["count"],
                item["command"],
            )
        )
    lines.append("Compaction candidates (by estimated savings):")
    if not compaction_candidates:
        lines.append("  (none)")
    else:
        cumulative = 0
        cumulative_bytes = 0
        for record in compaction_candidates[:top_n]:
            cumulative += record["count"]
            cumulative_bytes += record["estimated_saved_bytes"]
            command = (
                _redact_identifier(record["command"], mapping=command_aliases, prefix="cmd")
                if redact
                else record["command"]
            )
            lines.append(
                f"  {command}: {record['count']} invocations, "
                f"+~{record['estimated_saved_bytes']} bytes (~{record['estimated_saved_tokens']} tokens) "
                f"{((record['count']/total_invocations*100) if total_invocations else 0.0):.1f}%"
            )
        potential = (cumulative / total_invocations * 100) if total_invocations else 0.0
        lines.append(
            f"Cumulative impact top {min(top_n, len(compaction_candidates))}: "
            f"{cumulative} invocations ({potential:.1f}%), "
            f"~{cumulative_bytes} bytes (~{cumulative_bytes // ESTIMATED_BYTES_PER_TOKEN} tokens)"
        )

    unlisted_git = [
        record
        for record in report["unlisted_git_subcommands"]
        if record["coverage"] == "unlisted"
    ]
    lines.append("Unlisted git subcommands:")
    if unlisted_git:
        for record in sorted(unlisted_git, key=lambda item: (-item["count"], item["subcommand"])):
            lines.append(
                f"  {_redact_identifier(record['subcommand'], mapping=git_aliases, prefix='git') if redact else record['subcommand']}: {record['count']}"
            )
    else:
        lines.append("  (none)")
    lines.append(
        "Suggested next step: add the top candidate commands above to catalog ownership after a manual review."
    )
    return "\n".join(lines) + "\n"


def format_readable(
    report: dict[str, Any],
    *,
    include_noise: bool,
    excluded_commands: set[str] | frozenset[str],
    redact: bool = False,
    command_aliases: dict[str, str] | None = None,
    git_aliases: dict[str, str] | None = None,
) -> str:
    excluded_commands = {value.lower() for value in excluded_commands}
    command_aliases = command_aliases or {}
    git_aliases = git_aliases or {}
    lines = [f"Total invocations: {report['total_invocations']}", "Sources:"]
    lines.extend(f"  {source}: {count}" for source, count in sorted(report["sources"].items()))
    if excluded_commands and not include_noise:
        lines.append(
            "Excluded by default (use --include-noise to show): "
            + ", ".join(sorted(excluded_commands))
        )
    lines.append("Commands not covered by catalog:")
    unlisted = sorted(
        [
        record
        for record in report["commands"]
        if record["coverage"] == "unlisted"
        and (include_noise or record["command"].lower() not in excluded_commands)
    ],
        key=lambda item: (-item["count"], item["command"]),
    )
    if unlisted:
        lines.extend(
            f"  {_redact_identifier(record['command'], mapping=command_aliases, prefix='cmd') if redact else record['command']}: {record['count']} ({_format_sources(record.get('sources'))})"
            for record in unlisted
        )
    else:
        lines.append("  (none)")

    lines.append("Git subcommands not covered by catalog:")
    unlisted_git = sorted(
        [record for record in report["git_subcommands"] if record["coverage"] == "unlisted"],
        key=lambda item: (-item["count"], item["subcommand"]),
    )
    if unlisted_git:
        lines.extend(
            f"  {_redact_identifier(record['subcommand'], mapping=git_aliases, prefix='git') if redact else record['subcommand']}: {record['count']} ({_format_sources(record.get('sources'))})"
            for record in unlisted_git
        )
    else:
        lines.append("  (none)")
    return "\n".join(lines) + "\n"


def redact_report_output(report: dict[str, Any]) -> dict[str, Any]:
    command_aliases: dict[str, str] = {}
    git_aliases: dict[str, str] = {}
    def _redact(value: str) -> str:
        return _redact_identifier(value, mapping=command_aliases, prefix="cmd")

    def _redact_git(value: str) -> str:
        return _redact_identifier(value, mapping=git_aliases, prefix="git")

    return {
        "total_invocations": report["total_invocations"],
        "sources": report["sources"],
        "commands": [
            {
                "command": _redact(record["command"]),
                "count": record["count"],
                "coverage": record["coverage"],
                **({"sources": record.get("sources")} if "sources" in record else {}),
            }
            for record in report["commands"]
        ],
        "git_subcommands": [
            {
                "subcommand": _redact_git(record["subcommand"]),
                "count": record["count"],
                "coverage": record["coverage"],
                **({"sources": record.get("sources")} if "sources" in record else {}),
            }
            for record in report["git_subcommands"]
        ],
        "unlisted_commands": [
            {
                "command": _redact(record["command"]),
                "count": record["count"],
                "coverage": record["coverage"],
            }
            for record in report["unlisted_commands"]
        ],
        "unlisted_git_subcommands": [
            {
                "subcommand": _redact_git(record["subcommand"]),
                "count": record["count"],
                "coverage": record["coverage"],
            }
            for record in report["unlisted_git_subcommands"]
        ],
        "compaction_candidates": [
            {
                "command": _redact(record["command"]),
                "count": record["count"],
                "coverage": record["coverage"],
                "estimated_saved_bytes": record["estimated_saved_bytes"],
                "estimated_saved_invocations": record["estimated_saved_invocations"],
                "estimated_saved_tokens": record["estimated_saved_tokens"],
            }
            for record in report.get("compaction_candidates", [])
        ],
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--opencode-db", type=pathlib.Path, default=DEFAULT_OPENCODE_DB)
    parser.add_argument("--codex-root", type=pathlib.Path, default=DEFAULT_CODEX_ROOT)
    parser.add_argument("--claude-root", type=pathlib.Path, default=DEFAULT_CLAUDE_ROOT)
    parser.add_argument("--catalog", type=pathlib.Path, default=ROOT / "src/catalog.rs")
    parser.add_argument("--minimum", type=int, default=1)
    parser.add_argument("--top", type=int, default=10, help="Number of top compact candidates to show")
    parser.add_argument("--format", choices=("text", "json", "readable", "compact"), default="text")
    parser.add_argument(
        "--exclude-command",
        action="append",
        default=[],
        metavar="COMMAND",
        help="Exclude command from the default-noise filter",
    )
    parser.add_argument(
        "--exclude-command-file",
        type=pathlib.Path,
        default=DEFAULT_EXCLUDE_COMMAND_FILE,
        help="File with default excluded commands (one per line)",
    )
    parser.add_argument(
        "--compaction-metrics",
        type=pathlib.Path,
        default=None,
        help="JSONL file with optional per-command compaction telemetry",
    )
    parser.add_argument(
        "--include-noise",
        action="store_true",
        help="Include default-noise commands in missing-command output",
    )
    parser.add_argument(
        "--redact-output",
        action="store_true",
        help="Redact command and subcommand names in report output to avoid exposing potential PII.",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.minimum < 1:
        print("--minimum must be at least 1", file=sys.stderr)
        return 2
    if args.top < 1:
        print("--top must be at least 1", file=sys.stderr)
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
    excluded_commands = load_excluded_commands(args.exclude_command_file)
    compaction_metrics = load_compaction_metrics(args.compaction_metrics)
    excluded_commands.update(command.lower() for command in args.exclude_command)
    report = build_report(
        normalize_rows(raw_rows),
        catalog,
        args.minimum,
        compaction_metrics=compaction_metrics,
        readable=args.format in {"readable", "compact"},
        excluded_commands=excluded_commands,
        include_noise=args.include_noise,
    )
    if args.format == "json":
        output_report = redact_report_output(report) if args.redact_output else report
        print(json.dumps(output_report, indent=2, sort_keys=True))
    elif args.format == "compact":
        command_aliases: dict[str, str] = {}
        git_aliases: dict[str, str] = {}
        print(
            format_compaction_plan(
                report,
                top_n=args.top,
                include_noise=args.include_noise,
                excluded_commands=excluded_commands,
                redact=args.redact_output,
                command_aliases=command_aliases,
                git_aliases=git_aliases,
            ),
            end="",
        )
    elif args.format == "readable":
        command_aliases: dict[str, str] = {}
        git_aliases: dict[str, str] = {}
        print(
            format_readable(
                report,
                include_noise=args.include_noise,
                excluded_commands=excluded_commands,
                redact=args.redact_output,
                command_aliases=command_aliases,
                git_aliases=git_aliases,
            ),
            end="",
        )
    else:
        command_aliases: dict[str, str] = {}
        git_aliases: dict[str, str] = {}
        print(
            _format_text_with_redaction(
                report,
                redact=args.redact_output,
                command_aliases=command_aliases,
                git_aliases=git_aliases,
            ),
            end="",
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
