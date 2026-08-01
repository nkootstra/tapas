#!/usr/bin/env python3
"""Independently audit the pinned compatibility inventory and case coverage."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
from typing import Any


PINNED_COMMIT = "dbe73932586043f2d8e482df0e246c372125e1b2"
KNOWN_BENCHMARK_GAPS = {
    "aws", "biome", "brew", "df", "eslint", "jq", "lsof", "next",
    "psql", "systemctl", "tofu", "vitest", "zig",
}
REQUIRED_CAPABILITY_TYPES = {
    "claude_eligibility", "wrapper_dispatch", "git_subcommand", "pipe_detector",
    "transparent_runner", "exact_output_bypass", "stream_watch_policy", "generic_fallback",
}


def git(repo: pathlib.Path, *args: str) -> bytes:
    proc = subprocess.run(["git", "-C", str(repo), *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if proc.returncode:
        raise RuntimeError(proc.stderr.decode("utf-8", "replace").strip())
    return proc.stdout


def load(path: pathlib.Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def parse_catalog(source: str) -> set[str]:
    match = re.search(r"pub const auto_wrap_shell_case\s*=\s*(.*?);", source, re.S)
    if not match:
        raise RuntimeError("cannot independently parse auto-wrap catalog")
    strings = re.findall(r'"([^"\\]*(?:\\.[^"\\]*)*)"', match.group(1))
    return set("".join(strings).split("|"))


def parse_git(source: str) -> set[str]:
    match = re.search(r"const KnownSubcommand = enum\(u8\) \{(.*?)\};", source, re.S)
    if not match:
        raise RuntimeError("cannot independently parse Git dispatch")
    return {
        line.strip().rstrip(",").removeprefix('@"').removesuffix('"')
        for line in match.group(1).splitlines()
        if line.strip()
    }


def parse_pipe(source: str) -> list[str]:
    match = re.search(r"pub const Filters = \.\{(.*?)\};", source, re.S)
    if not match:
        raise RuntimeError("cannot independently parse pipe detector chain")
    body = re.sub(r"//.*", "", match.group(1))
    return [part.strip() for part in body.split(",") if part.strip()]


def validate_case(case: dict[str, Any], errors: list[str]) -> None:
    case_id = case.get("id", "<missing-id>")
    for key in ("mode", "argv", "env", "stdin", "expect", "covers"):
        if key not in case:
            errors.append(f"{case_id}: missing case field {key}")
    if not isinstance(case.get("argv"), list) or not case.get("argv"):
        errors.append(f"{case_id}: argv must be a non-empty array")
    env = case.get("env", {})
    if set(env) != {"set", "unset"}:
        errors.append(f"{case_id}: env must independently model set and unset")
    stdin = case.get("stdin", {})
    if set(stdin) != {"base64"}:
        errors.append(f"{case_id}: stdin must be represented as base64 bytes")
    expect = case.get("expect", {})
    if set(expect) != {"stdout", "stderr", "termination", "incomplete_output"}:
        errors.append(f"{case_id}: expectation must model streams, termination, and incomplete output")
        return
    for stream in ("stdout", "stderr"):
        if set(expect[stream]) != {"facts", "byte_exact"}:
            errors.append(f"{case_id}: {stream} must have independent facts and byte_exact")
    termination = expect["termination"]
    if set(termination) != {"exit_code", "signal"}:
        errors.append(f"{case_id}: termination must distinguish exit_code and signal")
    if (termination["exit_code"] is None) == (termination["signal"] is None):
        errors.append(f"{case_id}: exactly one exit_code or signal is required")
    if set(expect["incomplete_output"]) != {"expected", "diagnostic_facts"}:
        errors.append(f"{case_id}: incomplete-output diagnostics are not representable")


def benchmark_commands(path: pathlib.Path) -> set[str]:
    document = load(path)
    result: set[str] = set()
    for case in document.get("cases", []):
        command = case.get("command", [])
        if command:
            result.add(pathlib.PurePosixPath(command[0]).name)
    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=pathlib.Path, default=pathlib.Path("tests/compat/smll-v1.9.0"))
    parser.add_argument("--smll-repo", type=pathlib.Path)
    parser.add_argument("--benchmark-only", type=pathlib.Path, help="Prove a benchmark corpus is not the coverage authority")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    errors: list[str] = []
    inventory = load(args.contract / "inventory.json")
    cases_doc = load(args.contract / "cases.json")
    manifest = load(args.contract / "fixture-manifest.json")
    if inventory.get("source", {}).get("commit") != PINNED_COMMIT:
        errors.append("inventory is not pinned to the required smll commit")
    cases = cases_doc.get("cases", [])
    case_by_id = {case.get("id"): case for case in cases}
    if len(case_by_id) != len(cases):
        errors.append("case ids are not unique")
    for case in cases:
        validate_case(case, errors)

    capabilities = inventory.get("capabilities", [])
    capability_by_id = {cap.get("id"): cap for cap in capabilities}
    if len(capability_by_id) != len(capabilities):
        errors.append("capability ids are not unique")
    found_types = {cap.get("type") for cap in capabilities}
    missing_types = REQUIRED_CAPABILITY_TYPES - found_types
    if missing_types:
        errors.append(f"missing typed capability classes: {', '.join(sorted(missing_types))}")
    generic = [cap for cap in capabilities if cap.get("type") == "generic_fallback"]
    if len(generic) != 1:
        errors.append(f"generic fallback must be exactly one capability, found {len(generic)}")
    for capability in capabilities:
        capability_id = capability.get("id", "<missing-id>")
        anchors = capability.get("source_anchors", [])
        if not anchors or any("path" not in item or "line" not in item for item in anchors):
            errors.append(f"{capability_id}: missing source anchor")
        linked = capability.get("cases", [])
        if not linked:
            errors.append(f"{capability_id}: uncovered capability")
        for case_id in linked:
            if case_id not in case_by_id:
                errors.append(f"{capability_id}: unknown case {case_id}")
            elif capability_id not in case_by_id[case_id].get("covers", []):
                errors.append(f"{capability_id}: case {case_id} lacks reverse coverage link")

    manifest_entries = manifest.get("fixtures", [])
    expected_paths = {entry.get("path") for entry in manifest_entries}
    actual_paths = {
        str(path.relative_to(args.contract))
        for path in (args.contract / "fixtures").rglob("*") if path.is_file()
    }
    if expected_paths != actual_paths:
        for path in sorted(expected_paths - actual_paths):
            errors.append(f"missing fixture: {path}")
        for path in sorted(actual_paths - expected_paths):
            errors.append(f"extra fixture: {path}")
    for entry in manifest_entries:
        path = args.contract / entry["path"]
        if not path.is_file():
            continue
        data = path.read_bytes()
        if len(data) != entry.get("bytes") or hashlib.sha256(data).hexdigest() != entry.get("sha256"):
            errors.append(f"fixture hash/size mismatch: {entry['path']}")

    source_catalog: set[str] | None = None
    if args.smll_repo:
        repo = args.smll_repo.resolve()
        try:
            source_blobs = {
                item["path"]: git(repo, "show", f"{PINNED_COMMIT}:{item['path']}")
                for item in inventory.get("coverage_sources", [])
            }
            for source_record in inventory.get("coverage_sources", []):
                data = source_blobs[source_record["path"]]
                if hashlib.sha256(data).hexdigest() != source_record["sha256"]:
                    errors.append(f"coverage source hash mismatch: {source_record['path']}")
            source_catalog = parse_catalog(source_blobs["src/filter_catalog.zig"].decode())
            inventory_catalog = {cap["command"] for cap in capabilities if cap.get("type") == "claude_eligibility"}
            if source_catalog != inventory_catalog:
                errors.append(f"auto-wrap drift: missing={sorted(source_catalog - inventory_catalog)}, extra={sorted(inventory_catalog - source_catalog)}")
            source_git = parse_git(source_blobs["src/wrapper_git.zig"].decode())
            inventory_git = {cap["subcommand"] for cap in capabilities if cap.get("type") == "git_subcommand"}
            if len(source_git) != 18 or source_git != inventory_git:
                errors.append(f"Git dispatch drift: expected all 18, missing={sorted(source_git - inventory_git)}, extra={sorted(inventory_git - source_git)}")
            source_pipe = parse_pipe(source_blobs["src/pipe_filters.zig"].decode())
            inventory_pipe = [cap["detector"] for cap in sorted((cap for cap in capabilities if cap.get("type") == "pipe_detector"), key=lambda cap: cap["order"])]
            if source_pipe != inventory_pipe:
                errors.append("pipe detector membership or first-match order drifted")
            source_fixture_paths = {
                f"fixtures/{line}" for line in git(repo, "ls-tree", "-r", "--name-only", PINNED_COMMIT, "tests/fixtures", "benchmarks/smll-vs-rtk/fixtures").decode().splitlines()
            }
            if source_fixture_paths != expected_paths:
                errors.append("fixture manifest does not cover every pinned fixture blob")
        except (RuntimeError, KeyError, UnicodeDecodeError) as exc:
            errors.append(f"pinned-source audit failed: {exc}")

    if args.benchmark_only:
        benchmark = benchmark_commands(args.benchmark_only)
        catalog = source_catalog or {cap["command"] for cap in capabilities if cap.get("type") == "claude_eligibility"}
        missing = catalog - benchmark
        print(f"benchmark-only coverage missing {len(missing)} auto-wrap commands: {', '.join(sorted(missing))}")
        absent_known = KNOWN_BENCHMARK_GAPS - missing
        if absent_known:
            errors.append(f"historical benchmark no longer demonstrates known gaps: {', '.join(sorted(absent_known))}")
        else:
            errors.append("benchmark-only input is incomplete by design")

    if errors:
        print("inventory audit failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(f"inventory audit passed: {len(capabilities)} capabilities, {len(cases)} cases, {len(manifest_entries)} fixtures")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
