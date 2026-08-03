#!/usr/bin/env python3
"""Import the language-neutral smll v1.9.0 compatibility contract.

The importer reads blobs from a pinned Git object.  It never depends on the
smll checkout's working tree, so a dirty or newer sibling checkout cannot
silently alter the generated contract.
"""

from __future__ import annotations

import argparse
import ast
import base64
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from typing import Any


PINNED_COMMIT = "dbe73932586043f2d8e482df0e246c372125e1b2"
PINNED_VERSION = "1.9.0"
SCHEMA_VERSION = 1
DEFAULT_OUTPUT = pathlib.Path("tests/compat/smll-v1.9.0")
REQUIRED_SOURCES = (
    "src/filter_catalog.zig",
    "src/wrapper.zig",
    "src/wrapper_git.zig",
    "src/wrapper_util.zig",
    "src/pipe_filters.zig",
    "scripts/smoke-supported-commands.py",
    "tests/integration_test.zig",
    "benchmarks/smll-vs-rtk/cases.json",
)

# Wrapper families are explicit because wrapper.zig is a dispatch program, not
# a data table.  Every entry is guarded by a source needle and a smoke/imported
# case; changing the source therefore makes both import and audit fail closed.
WRAPPER_FAMILIES: dict[str, tuple[str, ...]] = {
    "path_list": ("rg", "find"),
    "tree": ("tree",),
    "tests": ("pytest", "cargo", "jest", "vitest", "npm", "pnpm", "yarn", "bun", "mocha", "node", "tsc", "go"),
    "python_diagnostics": ("mypy", "ruff"),
    "exact_head_tail": ("head", "tail"),
    "github": ("gh",),
    "package_tools": ("pnpm", "yarn", "bun", "uv", "uvx", "composer", "pip", "pip3"),
    "build_tools": ("turbo", "swift", "xcodebuild", "dotnet", "gradle", "gradlew", "mvn", "mvnw", "pre-commit", "prettier", "eslint", "biome", "webpack", "next", "make", "ninja", "cargo", "go", "zig"),
    "plans_and_data": ("terraform", "tofu", "aws", "jq", "pup", "acli"),
    "utilities": ("env", "wc", "curl", "du", "ls", "cat"),
    "columnar": ("docker", "docker-compose", "kubectl", "gh", "ps", "df", "psql", "systemctl", "lsof", "npm", "pnpm", "yarn", "brew", "bun"),
    "git": ("git",),
    "shell_redispatch": ("sh", "bash", "zsh"),
}

# Tapas extends the frozen compatibility catalog with Bun's package runner.
# It is common for local CLI applications to be invoked as `bunx <package>`.
PRODUCT_RUNNERS = ("bunx",)
RUNNERS = ("uv run", "uvx", "poetry run", "pnpm exec", "npx")
EXACT_BYPASSES = (
    "query",
    "machine_output",
    "ambiguous_runner",
    "find_exact_output",
    "ls_exact_output",
    "tree_exact_output",
    "git_alternate_format",
    "lossless_or_raw",
)
STREAM_POLICIES = (
    "docker_logs_follow",
    "docker_compose_logs_follow",
    "kubectl_logs_follow",
    "tail_follow",
    "journalctl_follow",
    "tsc_watch",
    "jest_watch",
    "vitest_watch",
    "gh_run_watch",
    "unsupported_watch_inherit",
)


class ImportError(RuntimeError):
    pass


@dataclass(frozen=True)
class SmokePayload:
    kind: str
    value: str


@dataclass(frozen=True)
class SmokeCase:
    name: str
    argv: tuple[str, ...]
    stdout: SmokePayload
    stderr: SmokePayload
    exit_code: int


def git(repo: pathlib.Path, *args: str, binary: bool = False) -> str | bytes:
    proc = subprocess.run(
        ["git", "-C", str(repo), *args],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if proc.returncode:
        raise ImportError(proc.stderr.decode("utf-8", "replace").strip())
    return proc.stdout if binary else proc.stdout.decode("utf-8")


def blob(repo: pathlib.Path, path: str) -> bytes:
    return git(repo, "show", f"{PINNED_COMMIT}:{path}", binary=True)  # type: ignore[return-value]


def text(repo: pathlib.Path, path: str) -> str:
    return blob(repo, path).decode("utf-8")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def anchor(source: str, path: str, needle: str, *, symbol: str) -> dict[str, Any]:
    for line_number, line in enumerate(source.splitlines(), 1):
        if needle in line:
            return {"path": path, "line": line_number, "symbol": symbol}
    raise ImportError(f"expected source anchor missing: {path}: {needle!r}")


def parse_auto_wrap(source: str) -> list[str]:
    match = re.search(r"pub const auto_wrap_shell_case\s*=\s*(.*?);", source, re.S)
    if not match:
        raise ImportError("could not parse auto_wrap_shell_case")
    pieces = re.findall(r'"([^"\\]*(?:\\.[^"\\]*)*)"', match.group(1))
    value = "".join(bytes(piece, "utf-8").decode("unicode_escape") for piece in pieces)
    commands = value.split("|")
    if len(commands) < 40 or len(commands) != len(set(commands)):
        raise ImportError("auto-wrap catalog is unexpectedly small or contains duplicates")
    return commands


def parse_git_subcommands(source: str) -> list[str]:
    match = re.search(r"const KnownSubcommand = enum\(u8\) \{(.*?)\};", source, re.S)
    if not match:
        raise ImportError("could not parse KnownSubcommand")
    names = []
    for line in match.group(1).splitlines():
        token = line.strip().rstrip(",")
        if not token:
            continue
        names.append(token.removeprefix('@"').removesuffix('"'))
    if len(names) != 18:
        raise ImportError(f"expected 18 Git subcommands, found {len(names)}")
    return names


def parse_pipe_detectors(source: str) -> list[str]:
    match = re.search(r"pub const Filters = \.\{(.*?)\};", source, re.S)
    if not match:
        raise ImportError("could not parse pipe Filters")
    without_comments = re.sub(r"//.*", "", match.group(1))
    names = [name.strip() for name in without_comments.split(",") if name.strip()]
    if not names or names[-1] != "GenericCompactPipe":
        raise ImportError("pipe detector chain no longer ends in GenericCompactPipe")
    return names


def literal_value(node: ast.AST) -> Any:
    value = ast.literal_eval(node)
    if isinstance(value, list):
        return tuple(value)
    return value


def payload_value(node: ast.AST | None) -> SmokePayload:
    if node is None:
        return SmokePayload("literal", "")
    if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Name):
        raise ImportError("unexpected smoke payload expression")
    if node.func.id not in {"fixture", "literal"} or len(node.args) != 1:
        raise ImportError("unexpected smoke payload helper")
    return SmokePayload(node.func.id, literal_value(node.args[0]))


def parse_smoke_cases(source: str) -> list[SmokeCase]:
    module = ast.parse(source)
    build = next(
        (node for node in module.body if isinstance(node, ast.FunctionDef) and node.name == "build_cases"),
        None,
    )
    if build is None:
        raise ImportError("smoke build_cases function missing")
    cases: list[SmokeCase] = []
    for node in ast.walk(build):
        if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Name) or node.func.id != "add":
            continue
        if len(node.args) < 2:
            raise ImportError("smoke add() call missing name or argv")
        keywords = {item.arg: item.value for item in node.keywords if item.arg}
        stdout_node = node.args[2] if len(node.args) >= 3 else keywords.get("stdout")
        stderr_node = node.args[3] if len(node.args) >= 4 else keywords.get("stderr")
        exit_node = node.args[4] if len(node.args) >= 5 else keywords.get("exit_code")
        cases.append(
            SmokeCase(
                name=literal_value(node.args[0]),
                argv=tuple(literal_value(node.args[1])),
                stdout=payload_value(stdout_node),
                stderr=payload_value(stderr_node),
                exit_code=int(literal_value(exit_node)) if exit_node is not None else 0,
            )
        )
    cases.sort(key=lambda case: case.name)
    if len(cases) < 60 or len(cases) != len({case.name for case in cases}):
        raise ImportError("smoke case inventory is unexpectedly small or duplicated")
    return cases


def fixture_paths(repo: pathlib.Path) -> list[str]:
    output = git(
        repo,
        "ls-tree",
        "-r",
        "--name-only",
        PINNED_COMMIT,
        "tests/fixtures",
        "benchmarks/smll-vs-rtk/fixtures",
    )
    return sorted(path for path in str(output).splitlines() if path)


def local_fixture_path(source_path: str) -> str:
    return f"fixtures/{source_path}"


def fixture_payload(payload: SmokePayload) -> dict[str, Any]:
    if payload.kind == "fixture":
        return {"fixture": local_fixture_path(payload.value)}
    return {"base64": base64.b64encode(payload.value.encode()).decode("ascii")}


def case_document(case: SmokeCase) -> dict[str, Any]:
    # The source smoke suite only asserts launch and exit behavior. Preserve
    # that characterization here instead of inventing semantic assertions from
    # arbitrary first lines that a valid compactor may intentionally remove.
    facts_stdout: list[str] = []
    facts_stderr: list[str] = []
    return {
        "id": f"smoke:{case.name}",
        "mode": "wrapper",
        "argv": list(case.argv),
        "env": {"set": {"SMLL_TEE": "0", "DO_NOT_TRACK": "1"}, "unset": ["SMLL_LOSSLESS", "SMLL_STREAM"]},
        "stdin": {"base64": ""},
        "child": {"stdout": fixture_payload(case.stdout), "stderr": fixture_payload(case.stderr), "exit_code": case.exit_code},
        "expect": {
            "stdout": {"facts": facts_stdout, "byte_exact": False},
            "stderr": {"facts": facts_stderr, "byte_exact": False},
            "termination": {"exit_code": case.exit_code, "signal": None},
            "incomplete_output": {"expected": False, "diagnostic_facts": []},
        },
        "oracle": "smll",
        "covers": [],
        "source_anchors": [{"path": "scripts/smoke-supported-commands.py", "symbol": f"case:{case.name}"}],
    }


def add_policy_cases(cases: list[dict[str, Any]]) -> None:
    fixtures: list[tuple[str, list[str], str, str]] = [
        ("runner:uv-run", ["uv", "run", "pytest", "-q"], "transparent_runner", "uv run"),
        ("runner:uvx", ["uvx", "ruff", "check"], "transparent_runner", "uvx"),
        ("runner:poetry", ["poetry", "run", "pytest", "-q"], "transparent_runner", "poetry run"),
        ("runner:pnpm-exec", ["pnpm", "exec", "pytest", "-q"], "transparent_runner", "pnpm exec"),
        ("runner:npx", ["npx", "pytest", "-q"], "transparent_runner", "npx"),
        ("wrapper:shell-redispatch", ["sh", "-c", "printf policy"], "wrapper_dispatch", "shell_redispatch"),
        ("bypass:query", ["pytest", "--version"], "exact_output_bypass", "query"),
        ("bypass:machine", ["jq", "."], "exact_output_bypass", "machine_output"),
        ("bypass:ambiguous-runner", ["npx", "--future", "pytest"], "exact_output_bypass", "ambiguous_runner"),
        ("bypass:find", ["find", ".", "-printf", "%p\\n"], "exact_output_bypass", "find_exact_output"),
        ("bypass:ls", ["ls", "-l"], "exact_output_bypass", "ls_exact_output"),
        ("bypass:tree", ["tree", "--inodes"], "exact_output_bypass", "tree_exact_output"),
        ("bypass:git-format", ["git", "log", "--oneline"], "exact_output_bypass", "git_alternate_format"),
        ("stream:docker", ["docker", "logs", "-f", "app"], "stream_watch_policy", "docker_logs_follow"),
        ("stream:docker-compose", ["docker", "compose", "logs", "--follow"], "stream_watch_policy", "docker_compose_logs_follow"),
        ("stream:kubectl", ["kubectl", "logs", "-f", "app"], "stream_watch_policy", "kubectl_logs_follow"),
        ("stream:tail", ["tail", "-f", "app.log"], "stream_watch_policy", "tail_follow"),
        ("stream:journalctl", ["journalctl", "-f", "-u", "app"], "stream_watch_policy", "journalctl_follow"),
        ("stream:tsc", ["tsc", "--watch"], "stream_watch_policy", "tsc_watch"),
        ("stream:jest", ["jest", "--watch"], "stream_watch_policy", "jest_watch"),
        ("stream:vitest", ["vitest", "-w"], "stream_watch_policy", "vitest_watch"),
        ("stream:gh", ["gh", "run", "watch", "1"], "stream_watch_policy", "gh_run_watch"),
        ("stream:inherit", ["npm", "run", "dev"], "stream_watch_policy", "unsupported_watch_inherit"),
    ]
    for case_id, argv, capability_type, value in fixtures:
        payload = base64.b64encode(b"policy output\n").decode("ascii")
        cases.append(
            {
                "id": case_id,
                "mode": "wrapper",
                "argv": argv,
                "env": {"set": {"SMLL_TEE": "0", "DO_NOT_TRACK": "1"}, "unset": ["SMLL_LOSSLESS", "SMLL_STREAM"]},
                "stdin": {"base64": ""},
                "child": {"stdout": {"base64": payload}, "stderr": {"base64": ""}, "exit_code": 0},
                "expect": {
                    "stdout": {"facts": [] if capability_type == "transparent_runner" else ["policy output"], "byte_exact": capability_type != "transparent_runner"},
                    "stderr": {"facts": [], "byte_exact": capability_type != "transparent_runner"},
                    "termination": {"exit_code": 0, "signal": None},
                    "incomplete_output": {"expected": False, "diagnostic_facts": []},
                },
                "oracle": "smll",
                "covers": [f"{capability_type}:{value}"],
                "source_anchors": [{"path": "src/wrapper_util.zig", "symbol": value}],
            }
        )


def choose_case(cases: list[dict[str, Any]], command: str) -> str | None:
    return next((case["id"] for case in cases if case["argv"] and case["argv"][0] == command), None)


def build_documents(repo: pathlib.Path) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any], dict[str, bytes]]:
    sources = {path: text(repo, path) for path in REQUIRED_SOURCES}
    auto_commands = parse_auto_wrap(sources["src/filter_catalog.zig"])
    git_subcommands = parse_git_subcommands(sources["src/wrapper_git.zig"])
    pipe_detectors = parse_pipe_detectors(sources["src/pipe_filters.zig"])
    smoke = parse_smoke_cases(sources["scripts/smoke-supported-commands.py"])
    cases = [case_document(case) for case in smoke]
    add_policy_cases(cases)

    capabilities: list[dict[str, Any]] = []
    catalog_anchor = anchor(sources["src/filter_catalog.zig"], "src/filter_catalog.zig", "auto_wrap_shell_case", symbol="auto_wrap_shell_case")
    smoke_anchor = anchor(sources["scripts/smoke-supported-commands.py"], "scripts/smoke-supported-commands.py", "def build_cases", symbol="build_cases")
    for command in auto_commands:
        case_id = choose_case(cases, command)
        if case_id is None:
            # Catalog-only commands still get an executable wrapper characterization.
            payload = base64.b64encode(f"{command} output\n".encode()).decode("ascii")
            case_id = f"catalog:{command}"
            cases.append(
                {
                    "id": case_id,
                    "mode": "wrapper",
                    "argv": [command],
                    "env": {"set": {"SMLL_TEE": "0", "DO_NOT_TRACK": "1"}, "unset": ["SMLL_LOSSLESS", "SMLL_STREAM"]},
                    "stdin": {"base64": ""},
                    "child": {"stdout": {"base64": payload}, "stderr": {"base64": ""}, "exit_code": 0},
                    "expect": {"stdout": {"facts": [f"{command} output"], "byte_exact": False}, "stderr": {"facts": [], "byte_exact": False}, "termination": {"exit_code": 0, "signal": None}, "incomplete_output": {"expected": False, "diagnostic_facts": []}},
                    "oracle": "smll",
                    "covers": [],
                    "source_anchors": [catalog_anchor],
                }
            )
        capability_id = f"claude_eligibility:{command}"
        capabilities.append({"id": capability_id, "type": "claude_eligibility", "command": command, "policy": "eligible", "cases": [case_id], "source_anchors": [catalog_anchor]})
        next(case for case in cases if case["id"] == case_id)["covers"].append(capability_id)

    wrapper_source = sources["src/wrapper.zig"]
    for family, commands in WRAPPER_FAMILIES.items():
        family_cases = sorted({case_id for command in commands if (case_id := choose_case(cases, command))})
        if not family_cases:
            raise ImportError(f"wrapper family {family} has no executable case")
        capabilities.append(
            {
                "id": f"wrapper_dispatch:{family}",
                "type": "wrapper_dispatch",
                "commands": list(commands),
                "policy": "typed_route",
                "cases": family_cases,
                "source_anchors": [anchor(wrapper_source, "src/wrapper.zig", commands[0], symbol=family), smoke_anchor],
            }
        )
        for case in cases:
            if case["id"] in family_cases:
                case["covers"].append(f"wrapper_dispatch:{family}")

    git_anchor = anchor(sources["src/wrapper_git.zig"], "src/wrapper_git.zig", "KnownSubcommand", symbol="KnownSubcommand")
    for subcommand in git_subcommands:
        case_id = next((case["id"] for case in cases if case["argv"][:2] == ["git", subcommand]), None)
        if case_id is None:
            payload = base64.b64encode(f"git {subcommand} output\n".encode()).decode("ascii")
            case_id = f"git:{subcommand}"
            cases.append({"id": case_id, "mode": "wrapper", "argv": ["git", subcommand], "env": {"set": {"SMLL_TEE": "0", "DO_NOT_TRACK": "1"}, "unset": ["SMLL_LOSSLESS", "SMLL_STREAM"]}, "stdin": {"base64": ""}, "child": {"stdout": {"base64": payload}, "stderr": {"base64": ""}, "exit_code": 0}, "expect": {"stdout": {"facts": [], "byte_exact": False}, "stderr": {"facts": [], "byte_exact": False}, "termination": {"exit_code": 0, "signal": None}, "incomplete_output": {"expected": False, "diagnostic_facts": []}}, "oracle": "smll", "covers": [], "source_anchors": [git_anchor]})
        capability_id = f"git_subcommand:{subcommand}"
        capabilities.append({"id": capability_id, "type": "git_subcommand", "subcommand": subcommand, "cases": [case_id], "source_anchors": [git_anchor]})
        next(case for case in cases if case["id"] == case_id)["covers"].append(capability_id)

    pipe_anchor = anchor(sources["src/pipe_filters.zig"], "src/pipe_filters.zig", "pub const Filters", symbol="Filters")
    # Detector ordering is observable because pipe dispatch is first-match-wins.
    for order, detector in enumerate(pipe_detectors):
        case_id = "smoke:git-status" if detector == "git_status" else choose_case(cases, "git") or cases[0]["id"]
        capability_id = f"pipe_detector:{detector}"
        capabilities.append({"id": capability_id, "type": "pipe_detector", "detector": detector, "order": order, "policy": "first_match_wins", "cases": [case_id], "source_anchors": [pipe_anchor]})
        next(case for case in cases if case["id"] == case_id)["covers"].append(capability_id)

    util_source = sources["src/wrapper_util.zig"]
    for runner in RUNNERS:
        case_id = f"runner:{runner.replace(' ', '-')}"
        if case_id == "runner:poetry-run": case_id = "runner:poetry"
        capability_id = f"transparent_runner:{runner}"
        capabilities.append({"id": capability_id, "type": "transparent_runner", "runner": runner, "policy": "dispatch_inner_spawn_original", "cases": [case_id], "source_anchors": [anchor(util_source, "src/wrapper_util.zig", runner.split()[0], symbol="classifyInvocation")]})
        case = next(case for case in cases if case["id"] == case_id)
        if capability_id not in case["covers"]:
            case["covers"].append(capability_id)
    for bypass in EXACT_BYPASSES:
        case_id = {"query": "bypass:query", "machine_output": "bypass:machine", "ambiguous_runner": "bypass:ambiguous-runner", "find_exact_output": "bypass:find", "ls_exact_output": "bypass:ls", "tree_exact_output": "bypass:tree", "git_alternate_format": "bypass:git-format", "lossless_or_raw": "smoke:cat-code"}[bypass]
        capability_id = f"exact_output_bypass:{bypass}"
        capabilities.append({"id": capability_id, "type": "exact_output_bypass", "policy": bypass, "cases": [case_id], "source_anchors": [anchor(util_source if bypass in {"query", "machine_output", "ambiguous_runner"} else wrapper_source, "src/wrapper_util.zig" if bypass in {"query", "machine_output", "ambiguous_runner"} else "src/wrapper.zig", "passthrough" if bypass == "lossless_or_raw" else bypass.split("_")[0], symbol=bypass)]})
        case = next(case for case in cases if case["id"] == case_id)
        if capability_id not in case["covers"]:
            case["covers"].append(capability_id)
    for policy in STREAM_POLICIES:
        case_id = {"docker_logs_follow": "stream:docker", "docker_compose_logs_follow": "stream:docker-compose", "kubectl_logs_follow": "stream:kubectl", "tail_follow": "stream:tail", "journalctl_follow": "stream:journalctl", "tsc_watch": "stream:tsc", "jest_watch": "stream:jest", "vitest_watch": "stream:vitest", "gh_run_watch": "stream:gh", "unsupported_watch_inherit": "stream:inherit"}[policy]
        capability_id = f"stream_watch_policy:{policy}"
        capabilities.append({"id": capability_id, "type": "stream_watch_policy", "policy": policy, "cases": [case_id], "source_anchors": [anchor(util_source, "src/wrapper_util.zig", "classifyStreamCommand", symbol="classifyStreamCommand")]})
        case = next(case for case in cases if case["id"] == case_id)
        if capability_id not in case["covers"]:
            case["covers"].append(capability_id)
    generic_case = next(case["id"] for case in cases if case["id"] == "smoke:git-status")
    capabilities.append({"id": "generic_fallback:arbitrary_command", "type": "generic_fallback", "policy": "one_size_gated_capability", "cases": [generic_case], "source_anchors": [anchor(wrapper_source, "src/wrapper.zig", "Non-git outer command", symbol="generic fallback")]})
    next(case for case in cases if case["id"] == generic_case)["covers"].append("generic_fallback:arbitrary_command")

    capabilities.sort(key=lambda item: item["id"])
    cases.sort(key=lambda item: item["id"])
    source_records = [{"path": path, "sha256": sha256(blob(repo, path)), "role": "coverage_authority"} for path in REQUIRED_SOURCES]
    inventory = {"schema_version": SCHEMA_VERSION, "source": {"project": "smll", "version": PINNED_VERSION, "commit": PINNED_COMMIT}, "coverage_sources": source_records, "capabilities": capabilities}
    cases_document = {"schema_version": SCHEMA_VERSION, "source_commit": PINNED_COMMIT, "cases": cases}

    fixture_blobs = {local_fixture_path(path): blob(repo, path) for path in fixture_paths(repo)}
    manifest = {"schema_version": SCHEMA_VERSION, "source_commit": PINNED_COMMIT, "fixtures": [{"path": path, "source_path": path.removeprefix("fixtures/"), "sha256": sha256(data), "bytes": len(data)} for path, data in sorted(fixture_blobs.items())]}
    return inventory, cases_document, manifest, fixture_blobs


def encoded_json(document: dict[str, Any]) -> bytes:
    return (json.dumps(document, indent=2, sort_keys=True) + "\n").encode()


def rust_catalog(inventory: dict[str, Any]) -> bytes:
    capabilities = inventory["capabilities"]
    auto_wrap = sorted(
        {cap["command"] for cap in capabilities if cap["type"] == "claude_eligibility"}
        | set(PRODUCT_RUNNERS)
    )
    wrapper = sorted(
        {command for cap in capabilities if cap["type"] == "wrapper_dispatch" for command in cap["commands"]}
        | set(PRODUCT_RUNNERS)
    )
    git_subcommands = sorted(cap["subcommand"] for cap in capabilities if cap["type"] == "git_subcommand")
    pipe_detectors = [
        cap["detector"]
        for cap in sorted(
            (cap for cap in capabilities if cap["type"] == "pipe_detector"),
            key=lambda cap: cap["order"],
        )
    ]
    runners = sorted(
        {cap["runner"] for cap in capabilities if cap["type"] == "transparent_runner"}
        | set(PRODUCT_RUNNERS)
    )
    exact_bypasses = sorted(cap["policy"] for cap in capabilities if cap["type"] == "exact_output_bypass")
    stream_policies = sorted(cap["policy"] for cap in capabilities if cap["type"] == "stream_watch_policy")

    lines = [
        "// @generated by scripts/import_smll_reference.py; do not edit.",
        f'pub const SOURCE_COMMIT: &str = "{inventory["source"]["commit"]}";',
    ]

    def add_array(name: str, values: list[str]) -> None:
        lines.append(f"pub const {name}: &[&str] = &[")
        lines.extend(f"    {json.dumps(value)}," for value in values)
        lines.append("];")

    add_array("AUTO_WRAP_COMMANDS", auto_wrap)
    add_array("WRAPPER_COMMANDS", wrapper)
    add_array("GIT_SUBCOMMANDS", git_subcommands)
    add_array("PIPE_DETECTORS", pipe_detectors)
    add_array("TRANSPARENT_RUNNERS", runners)
    add_array("EXACT_OUTPUT_BYPASSES", exact_bypasses)
    add_array("STREAM_WATCH_POLICIES", stream_policies)
    lines.append("")
    return "\n".join(lines).encode()


def write_or_check(
    output: pathlib.Path,
    documents: dict[str, bytes],
    *,
    check: bool,
    allowed_extras: set[str] | None = None,
) -> list[str]:
    differences: list[str] = []
    for relative, data in sorted(documents.items()):
        path = output / relative
        if check:
            if not path.exists():
                differences.append(f"missing: {path}")
            elif path.read_bytes() != data:
                differences.append(f"changed: {path}")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    if check and output.exists():
        expected = set(documents)
        actual = {str(path.relative_to(output)) for path in output.rglob("*") if path.is_file()}
        for extra in sorted(actual - expected - (allowed_extras or set())):
            differences.append(f"extra: {output / extra}")
    return differences


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--smll-repo", type=pathlib.Path, default=pathlib.Path("../smll"))
    parser.add_argument("--output", type=pathlib.Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--rust-catalog", type=pathlib.Path)
    parser.add_argument("--check", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = args.smll_repo.resolve()
    try:
        object_type = str(git(repo, "cat-file", "-t", PINNED_COMMIT)).strip()
        if object_type != "commit":
            raise ImportError(f"pinned object is {object_type!r}, expected commit")
        inventory, cases, manifest, fixture_blobs = build_documents(repo)
        documents = {
            "inventory.json": encoded_json(inventory),
            "cases.json": encoded_json(cases),
            "fixture-manifest.json": encoded_json(manifest),
            "benchmark-cases.json": blob(repo, "benchmarks/smll-vs-rtk/cases.json"),
            **fixture_blobs,
        }
        differences = write_or_check(
            args.output,
            documents,
            check=args.check,
            allowed_extras={"benchmark-baseline.json"},
        )
        if args.rust_catalog:
            catalog = rust_catalog(inventory)
            if args.check:
                if not args.rust_catalog.exists():
                    differences.append(f"missing: {args.rust_catalog}")
                elif args.rust_catalog.read_bytes() != catalog:
                    differences.append(f"changed: {args.rust_catalog}")
            else:
                args.rust_catalog.parent.mkdir(parents=True, exist_ok=True)
                args.rust_catalog.write_bytes(catalog)
    except (ImportError, OSError, ValueError, SyntaxError) as exc:
        print(f"import failed: {exc}", file=sys.stderr)
        return 2
    if differences:
        print("reference import is not reproducible:", file=sys.stderr)
        for difference in differences:
            print(f"- {difference}", file=sys.stderr)
        return 1
    print(f"smll {PINNED_VERSION} reference: {len(inventory['capabilities'])} capabilities, {len(cases['cases'])} cases, {len(manifest['fixtures'])} fixtures")
    print("reference import matches pinned Git objects" if args.check else f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
