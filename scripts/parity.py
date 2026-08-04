#!/usr/bin/env python3
"""Run language-neutral compatibility cases against smll or Tapas binaries."""

from __future__ import annotations

import argparse
import base64
import concurrent.futures
import json
import os
import pathlib
import shlex
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from typing import Any


DEFAULT_CASES = pathlib.Path("tests/regression/cases.json")
DEFAULT_CONTRACT = pathlib.Path("tests/regression")


@dataclass
class Result:
    case_id: str
    returncode: int
    stdout: bytes
    stderr: bytes
    errors: list[str]


def bytes_value(value: dict[str, str], contract: pathlib.Path) -> bytes:
    if "base64" in value:
        return base64.b64decode(value["base64"], validate=True)
    if "fixture" in value:
        return (contract / value["fixture"]).read_bytes()
    raise ValueError("byte value must contain base64 or fixture")


def fixture_paths(value: Any) -> list[str]:
    paths: list[str] = []
    if isinstance(value, dict):
        for key, item in value.items():
            if key == "fixture" and isinstance(item, str):
                paths.append(item)
            paths.extend(fixture_paths(item))
    elif isinstance(value, list):
        for item in value:
            paths.extend(fixture_paths(item))
    return paths


def missing_fixtures(cases: list[dict[str, Any]], contract: pathlib.Path) -> list[str]:
    return sorted(
        {
            path
            for case in cases
            for path in fixture_paths(case)
            if not (contract / path).is_file()
        }
    )


def fake_tool(bin_dir: pathlib.Path, command: str, child: dict[str, Any], contract: pathlib.Path) -> None:
    fake_tool_bytes(
        bin_dir,
        command,
        bytes_value(child["stdout"], contract),
        bytes_value(child["stderr"], contract),
        int(child["exit_code"]),
    )


def fake_tool_bytes(
    bin_dir: pathlib.Path,
    command: str,
    stdout: bytes,
    stderr: bytes,
    exit_code: int,
) -> None:
    stdout_path = bin_dir / "child.stdout"
    stderr_path = bin_dir / "child.stderr"
    stdout_path.write_bytes(stdout)
    stderr_path.write_bytes(stderr)
    tool = bin_dir / pathlib.PurePosixPath(command).name
    script = "\n".join(
        (
            "#!/bin/sh",
            f"/bin/cat {shlex.quote(str(stdout_path))}",
            f"/bin/cat {shlex.quote(str(stderr_path))} >&2",
            f"exit {exit_code}",
            "",
        )
    )
    tool.write_text(script, encoding="utf-8")
    tool.chmod(0o755)


def assert_result(case: dict[str, Any], proc: subprocess.CompletedProcess[bytes], child: dict[str, Any], contract: pathlib.Path) -> list[str]:
    errors: list[str] = []
    expected = case["expect"]
    termination = expected["termination"]
    if termination["exit_code"] is not None and proc.returncode != termination["exit_code"]:
        errors.append(f"exit {proc.returncode} != {termination['exit_code']}")
    if termination["signal"] is not None and proc.returncode != -termination["signal"]:
        errors.append(f"signal result {proc.returncode} != {-termination['signal']}")
    for stream_name in ("stdout", "stderr"):
        actual = getattr(proc, stream_name)
        stream_expect = expected[stream_name]
        for fact in stream_expect["facts"]:
            if fact.encode("utf-8") not in actual:
                errors.append(f"{stream_name} missing fact {fact!r}")
        if stream_expect["byte_exact"]:
            raw = bytes_value(child[stream_name], contract)
            if actual != raw:
                errors.append(f"{stream_name} is not byte-exact ({len(actual)} != {len(raw)} bytes)")
    for fact in expected["incomplete_output"]["diagnostic_facts"]:
        if fact.encode("utf-8") not in proc.stderr:
            errors.append(f"stderr missing incomplete-output fact {fact!r}")
    return errors


def run_case(binary: pathlib.Path, case: dict[str, Any], contract: pathlib.Path, timeout: float, tool_kind: str) -> Result:
    with tempfile.TemporaryDirectory(prefix="tapas-parity-") as temp_name:
        temp = pathlib.Path(temp_name)
        bin_dir = temp / "bin"
        bin_dir.mkdir()
        child = case.get("child", {"stdout": {"base64": ""}, "stderr": {"base64": ""}, "exit_code": 0})
        fake_tool(bin_dir, case["argv"][0], child, contract)
        env = dict(os.environ)
        env["PATH"] = str(bin_dir) + os.pathsep + env.get("PATH", "")
        env["HOME"] = str(temp / "home")
        pathlib.Path(env["HOME"]).mkdir()
        env["DO_NOT_TRACK"] = "1"
        env[("SMLL" if tool_kind == "smll" else "TAPAS") + "_TEE"] = "0"
        for key, value in case["env"]["set"].items():
            mapped = key.replace("SMLL_", "TAPAS_", 1) if tool_kind == "tapas" else key
            env[mapped] = value
        for key in case["env"]["unset"]:
            mapped = key.replace("SMLL_", "TAPAS_", 1) if tool_kind == "tapas" else key
            env.pop(mapped, None)
        if case["mode"] == "wrapper":
            argv = [str(binary), *case["argv"]]
        elif case["mode"] == "pipe":
            argv = [str(binary)]
        else:
            return Result(case["id"], 2, b"", b"", [f"unsupported mode {case['mode']!r}"])
        try:
            proc = subprocess.run(
                argv,
                input=bytes_value(case["stdin"], contract),
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=env,
                timeout=timeout,
                check=False,
            )
        except subprocess.TimeoutExpired:
            return Result(case["id"], 124, b"", b"", [f"timed out after {timeout:g}s"])
        return Result(case["id"], proc.returncode, proc.stdout, proc.stderr, assert_result(case, proc, child, contract))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--tool", choices=("smll", "tapas"), required=True)
    parser.add_argument("--cases", type=pathlib.Path, default=DEFAULT_CASES)
    parser.add_argument("--contract", type=pathlib.Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--case", action="append", dest="case_ids")
    parser.add_argument(
        "--jobs",
        type=int,
        default=min(8, os.cpu_count() or 1),
        help="number of isolated cases to run concurrently (default: %(default)s)",
    )
    parser.add_argument("--timeout", type=float, default=10.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.jobs < 1:
        print("--jobs must be at least 1", file=sys.stderr)
        return 2
    document = json.loads(args.cases.read_text(encoding="utf-8"))
    cases = [case for case in document["cases"] if case.get("oracle") == "smll" or args.tool == "tapas"]
    if args.case_ids:
        requested = set(args.case_ids)
        known = {case["id"] for case in cases}
        if missing := sorted(requested - known):
            print(f"unknown cases: {', '.join(missing)}", file=sys.stderr)
            return 2
        cases = [case for case in cases if case["id"] in requested]
    missing = missing_fixtures(cases, args.contract)
    if missing:
        print("missing regression fixtures:", file=sys.stderr)
        for path in missing:
            print(f"- {path}", file=sys.stderr)
        return 2
    binary = args.binary.resolve()
    if not binary.is_file():
        print(f"binary not found: {binary}", file=sys.stderr)
        return 2
    run = lambda case: run_case(binary, case, args.contract, args.timeout, args.tool)
    if args.jobs == 1:
        results = map(run, cases)
        failures = [result for result in results if result.errors]
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as executor:
            failures = [result for result in executor.map(run, cases) if result.errors]
    print(f"{args.tool} parity cases: {len(cases)}")
    if failures:
        for result in failures:
            print(f"FAIL {result.case_id}: {'; '.join(result.errors)}", file=sys.stderr)
        return 1
    print("all characterization cases passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
