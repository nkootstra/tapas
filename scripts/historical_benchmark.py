#!/usr/bin/env python3
"""Run Tapas against smll's pinned 94-case CLI token corpus."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
from typing import Any

import benchmark
import parity


DEFAULT_CONTRACT = pathlib.Path("tests/compat/smll-v1.9.0")
DEFAULT_BASELINE = DEFAULT_CONTRACT / "benchmark-baseline.json"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fixture_bytes(contract: pathlib.Path, source_path: str) -> bytes:
    return (contract / "fixtures" / source_path).read_bytes()


def streams(case: dict[str, Any], contract: pathlib.Path) -> tuple[bytes, bytes]:
    stdout = fixture_bytes(contract, case["fixture"])
    stderr = fixture_bytes(contract, case["stderr_fixture"]) if case.get("stderr_fixture") else b""
    if case.get("stream") == "stderr":
        stdout, stderr = b"", stdout
    return stdout, stderr


def run_case(
    binary: pathlib.Path,
    tool: str,
    case: dict[str, Any],
    contract: pathlib.Path,
    timeout: float,
) -> subprocess.CompletedProcess[bytes]:
    stdout, stderr = streams(case, contract)
    child = {
        "stdout": {"base64": base64.b64encode(stdout).decode("ascii")},
        "stderr": {"base64": base64.b64encode(stderr).decode("ascii")},
        "exit_code": case.get("exit_code", 0),
    }
    with tempfile.TemporaryDirectory(prefix="tapas-benchmark-") as temp_name:
        temp = pathlib.Path(temp_name)
        bin_dir = temp / "bin"
        bin_dir.mkdir()
        parity.fake_tool(bin_dir, case["command"][0], child, contract)
        home = temp / "home"
        home.mkdir()
        env = dict(os.environ)
        env["PATH"] = str(bin_dir) + os.pathsep + env.get("PATH", "")
        env["HOME"] = str(home)
        env["DO_NOT_TRACK"] = "1"
        prefix = "SMLL" if tool == "smll" else "TAPAS"
        env[prefix + "_TEE"] = "0"
        env.pop(prefix + "_LOSSLESS", None)
        env.pop(prefix + "_STREAM", None)
        return subprocess.run(
            [binary.resolve(), *case["command"]],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            timeout=timeout,
            check=False,
        )


def case_metrics(
    proc: subprocess.CompletedProcess[bytes],
    encoder: Any,
    elapsed_ns: int,
) -> dict[str, Any]:
    combined = proc.stdout + proc.stderr
    stdout_tokens = benchmark.tokens(encoder, proc.stdout)
    stderr_tokens = benchmark.tokens(encoder, proc.stderr)
    return {
        "exit_result": proc.returncode,
        "stdout_bytes": len(proc.stdout),
        "stderr_bytes": len(proc.stderr),
        "combined_sha256": digest(combined),
        "proxy_tokens": (
            None
            if stdout_tokens is None or stderr_tokens is None
            else stdout_tokens + stderr_tokens
        ),
        "elapsed_ns": elapsed_ns,
    }


def declared_facts(case: dict[str, Any]) -> list[str]:
    return case.get("tool_signals", {}).get("smll", case.get("signals", []))


def build_baseline(
    binary: pathlib.Path,
    cases: list[dict[str, Any]],
    corpus: bytes,
    contract: pathlib.Path,
    encoder: Any,
    timeout: float,
) -> dict[str, Any]:
    records = []
    for case in cases:
        started = time.perf_counter_ns()
        proc = run_case(binary, "smll", case, contract, timeout)
        metrics = case_metrics(proc, encoder, time.perf_counter_ns() - started)
        metrics.pop("elapsed_ns")
        combined = proc.stdout + proc.stderr
        metrics.update(
            {
                "name": case["name"],
                "command": case["command"],
                "visible_facts": [
                    fact for fact in declared_facts(case) if fact.encode("utf-8") in combined
                ],
            }
        )
        records.append(metrics)
    return {
        "schema_version": 1,
        "source_commit": "dbe73932586043f2d8e482df0e246c372125e1b2",
        "corpus_sha256": digest(corpus),
        "tokenizer_sha256": benchmark.TOKENIZER_HASH,
        "smll_binary_sha256": digest(binary.read_bytes()),
        "records": records,
    }


def compare(
    binary: pathlib.Path,
    cases: list[dict[str, Any]],
    corpus: bytes,
    baseline: dict[str, Any],
    contract: pathlib.Path,
    encoder: Any,
    timeout: float,
) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    if baseline.get("corpus_sha256") != digest(corpus):
        errors.append("benchmark corpus does not match the pinned baseline")
    if baseline.get("tokenizer_sha256") != benchmark.TOKENIZER_HASH:
        errors.append("benchmark tokenizer does not match the pinned baseline")
    expected = {record["name"]: record for record in baseline.get("records", [])}
    if set(expected) != {case["name"] for case in cases}:
        errors.append("benchmark baseline case membership differs from the corpus")

    records = []
    total_raw_tokens = 0
    total_tapas_tokens = 0
    exact = 0
    for case in cases:
        stdout, stderr = streams(case, contract)
        raw_stdout_tokens = benchmark.tokens(encoder, stdout)
        raw_stderr_tokens = benchmark.tokens(encoder, stderr)
        raw_tokens = (
            None
            if raw_stdout_tokens is None or raw_stderr_tokens is None
            else raw_stdout_tokens + raw_stderr_tokens
        )
        started = time.perf_counter_ns()
        proc = run_case(binary, "tapas", case, contract, timeout)
        metrics = case_metrics(proc, encoder, time.perf_counter_ns() - started)
        oracle = expected.get(case["name"], {})
        combined = proc.stdout + proc.stderr
        missing_facts = [
            fact
            for fact in oracle.get("visible_facts", [])
            if fact.encode("utf-8") not in combined
        ]
        if metrics["exit_result"] != oracle.get("exit_result"):
            errors.append(
                f"{case['name']}: exit {metrics['exit_result']} != {oracle.get('exit_result')}"
            )
        if metrics["combined_sha256"] == oracle.get("combined_sha256"):
            exact += 1
        else:
            errors.append(f"{case['name']}: combined output differs from the smll baseline")
        if missing_facts:
            errors.append(f"{case['name']}: missing oracle-visible facts: {', '.join(missing_facts)}")
        if (
            metrics["proxy_tokens"] is not None
            and oracle.get("proxy_tokens") is not None
            and metrics["proxy_tokens"] > oracle["proxy_tokens"]
        ):
            errors.append(
                f"{case['name']}: token regression {metrics['proxy_tokens']} > {oracle['proxy_tokens']}"
            )
        if raw_tokens is not None and metrics["proxy_tokens"] is not None:
            total_raw_tokens += raw_tokens
            total_tapas_tokens += metrics["proxy_tokens"]
        records.append(
            {
                "name": case["name"],
                "category": case["category"],
                "raw_proxy_tokens": raw_tokens,
                "tapas": metrics,
                "smll_proxy_tokens": oracle.get("proxy_tokens"),
                "missing_facts": missing_facts,
            }
        )
    savings = (
        0
        if total_raw_tokens == 0
        else (total_raw_tokens - total_tapas_tokens) * 100 / total_raw_tokens
    )
    return (
        {
            "schema_version": 1,
            "cases": len(cases),
            "exact_combined": exact,
            "raw_proxy_tokens": total_raw_tokens,
            "tapas_proxy_tokens": total_tapas_tokens,
            "proxy_token_savings_percent": round(savings, 2),
            "records": records,
        },
        errors,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tapas-bin", type=pathlib.Path)
    parser.add_argument("--smll-bin", type=pathlib.Path)
    parser.add_argument("--contract", type=pathlib.Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--cases", type=pathlib.Path)
    parser.add_argument("--baseline", type=pathlib.Path, default=DEFAULT_BASELINE)
    parser.add_argument("--tokenizer", type=pathlib.Path, default=pathlib.Path("tests/tokenizers/o200k_base.tiktoken"))
    parser.add_argument("--write-baseline", action="store_true")
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--timeout", type=float, default=10.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    cases_path = args.cases or args.contract / "benchmark-cases.json"
    corpus = cases_path.read_bytes()
    cases = json.loads(corpus)["cases"]
    try:
        encoder = benchmark.load_encoder(args.tokenizer)
    except (OSError, ValueError, RuntimeError) as error:
        print(f"benchmark setup failed: {error}", file=sys.stderr)
        return 2

    if args.write_baseline:
        if args.smll_bin is None:
            print("--write-baseline requires --smll-bin", file=sys.stderr)
            return 2
        baseline = build_baseline(
            args.smll_bin, cases, corpus, args.contract, encoder, args.timeout
        )
        args.baseline.write_text(
            json.dumps(baseline, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        print(f"wrote {args.baseline} with {len(cases)} pinned smll cases")
        return 0

    if args.tapas_bin is None:
        print("benchmark requires --tapas-bin", file=sys.stderr)
        return 2
    baseline = json.loads(args.baseline.read_bytes())
    report, errors = compare(
        args.tapas_bin,
        cases,
        corpus,
        baseline,
        args.contract,
        encoder,
        args.timeout,
    )
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    print(
        f"historical benchmark: {report['exact_combined']}/{report['cases']} exact; "
        f"tokens {report['raw_proxy_tokens']} -> {report['tapas_proxy_tokens']} "
        f"({report['proxy_token_savings_percent']}% saved)"
    )
    for error in errors:
        print(f"FAIL {error}", file=sys.stderr)
    return int(bool(errors))


if __name__ == "__main__":
    raise SystemExit(main())
