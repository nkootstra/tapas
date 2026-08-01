#!/usr/bin/env python3
"""Measure release artifacts and enforce the checked-in release budget."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import pathlib
import re
import statistics
import subprocess
import sys
import time
from typing import Any


LINUX_RSS_MARKER = b"TAPAS_MAX_RSS_KIB="


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def artifact_metrics(path: pathlib.Path) -> dict[str, Any]:
    if not path.is_file():
        raise ValueError(f"artifact not found: {path}")
    data = path.read_bytes()
    return {
        "sha256": sha256(data),
        "uncompressed_bytes": len(data),
        "gzip_bytes": len(gzip.compress(data, compresslevel=9, mtime=0)),
    }


def parse_profile(value: str) -> tuple[str, pathlib.Path]:
    name, separator, raw_path = value.partition("=")
    if not separator or not name or not raw_path:
        raise argparse.ArgumentTypeError("profiles must use NAME=PATH")
    if not re.fullmatch(r"[A-Za-z0-9._-]+", name):
        raise argparse.ArgumentTypeError(f"invalid profile name: {name}")
    return name, pathlib.Path(raw_path)


def command_environment() -> dict[str, str]:
    env = dict(os.environ)
    env.update({"DO_NOT_TRACK": "1", "LC_ALL": "C", "TAPAS_TEE": "0"})
    env.pop("TAPAS_LOSSLESS", None)
    env.pop("TAPAS_STREAM", None)
    return env


def checked_run(
    command: list[pathlib.Path | str],
    *,
    input_bytes: bytes | None,
    timeout: float,
) -> tuple[subprocess.CompletedProcess[bytes], int]:
    started = time.perf_counter_ns()
    result = subprocess.run(
        [str(part) for part in command],
        input=input_bytes,
        stdin=subprocess.DEVNULL if input_bytes is None else None,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=command_environment(),
        timeout=timeout,
        check=False,
    )
    elapsed_ns = time.perf_counter_ns() - started
    if result.returncode != 0:
        stderr = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"command exited {result.returncode}: {stderr}")
    return result, elapsed_ns


def measure_startup(binary: pathlib.Path, samples: int, timeout: float) -> dict[str, Any]:
    if samples < 1:
        raise ValueError("startup samples must be positive")
    checked_run([binary, "--version"], input_bytes=None, timeout=timeout)
    elapsed = [
        checked_run([binary, "--version"], input_bytes=None, timeout=timeout)[1]
        for _ in range(samples)
    ]
    return {
        "status": "measured",
        "samples": samples,
        "median_ns": int(statistics.median(elapsed)),
        "min_ns": min(elapsed),
        "max_ns": max(elapsed),
    }


def pipe_payload(size: int) -> bytes:
    if size < 1:
        raise ValueError("pipe input size must be positive")
    line = b"src/main.rs:42:7: error: deterministic release measurement\n"
    return (line * ((size + len(line) - 1) // len(line)))[:size]


def measure_pipe(binary: pathlib.Path, payload: bytes, timeout: float) -> dict[str, Any]:
    result, elapsed_ns = checked_run([binary], input_bytes=payload, timeout=timeout)
    return {
        "status": "measured",
        "input_bytes": len(payload),
        "input_sha256": sha256(payload),
        "output_bytes": len(result.stdout),
        "elapsed_ns": elapsed_ns,
        "bytes_per_second": int(len(payload) * 1_000_000_000 / max(elapsed_ns, 1)),
    }


def parse_linux_rss(stderr: bytes) -> int:
    match = re.search(rb"(?:^|\n)TAPAS_MAX_RSS_KIB=(\d+)(?:\n|$)", stderr)
    if match is None:
        raise ValueError("GNU time output did not contain the peak RSS marker")
    return int(match.group(1)) * 1024


def parse_macos_rss(stderr: bytes) -> int:
    match = re.search(rb"(?:^|\n)\s*(\d+)\s+maximum resident set size(?:\n|$)", stderr)
    if match is None:
        raise ValueError("macOS time output did not contain maximum resident set size")
    return int(match.group(1))


def measure_peak_rss(binary: pathlib.Path, payload: bytes, timeout: float) -> dict[str, Any]:
    time_binary = pathlib.Path("/usr/bin/time")
    if not time_binary.is_file():
        raise RuntimeError("/usr/bin/time is unavailable")
    if sys.platform.startswith("linux"):
        command: list[pathlib.Path | str] = [
            time_binary,
            "-f",
            LINUX_RSS_MARKER.decode("ascii") + "%M",
            binary,
        ]
        provider = "gnu-time-kib"
        parser = parse_linux_rss
    elif sys.platform == "darwin":
        command = [time_binary, "-l", binary]
        provider = "macos-time-bytes"
        parser = parse_macos_rss
    else:
        raise RuntimeError(f"peak RSS measurement is unsupported on {sys.platform}")
    result, _ = checked_run(command, input_bytes=payload, timeout=timeout)
    return {"status": "measured", "provider": provider, "bytes": parser(result.stderr)}


def unavailable(error: Exception) -> dict[str, str]:
    return {"status": "unavailable", "reason": str(error)}


def resolve_metric(document: dict[str, Any], metric: str) -> tuple[bool, Any]:
    value: Any = document
    for part in metric.split("."):
        if not isinstance(value, dict) or part not in value:
            return False, None
        if value.get("status") == "unavailable":
            return False, None
        value = value[part]
    return value is not None, value


def evaluate_policy(
    report: dict[str, Any], policy: dict[str, Any], target: str
) -> dict[str, Any]:
    targets = policy.get("targets")
    if not isinstance(targets, dict) or target not in targets:
        raise ValueError(f"no release budget for target: {target}")
    target_policy = targets[target]
    required = policy.get("required_measurements")
    hard_caps = (
        target_policy.get("hard_caps") if isinstance(target_policy, dict) else None
    )
    if not isinstance(required, list) or not all(isinstance(item, str) for item in required):
        raise ValueError("policy required_measurements must be a list of metric names")
    if not isinstance(hard_caps, dict) or not hard_caps:
        raise ValueError(f"target {target} must define at least one hard cap")

    required_evidence = []
    for metric in required:
        available, value = resolve_metric(report, metric)
        required_evidence.append(
            {
                "metric": metric,
                "available": available,
                "value": value if available else None,
            }
        )

    cap_evidence = []
    for metric, limit in sorted(hard_caps.items()):
        if not isinstance(limit, int) or isinstance(limit, bool) or limit < 1:
            raise ValueError(f"hard cap for {metric} must be a positive integer")
        available, actual = resolve_metric(report, metric)
        passed = available and isinstance(actual, (int, float)) and actual <= limit
        cap_evidence.append(
            {
                "metric": metric,
                "actual": actual if available else None,
                "limit": limit,
                "result": "pass" if passed else "fail",
            }
        )

    passed = all(item["available"] for item in required_evidence) and all(
        item["result"] == "pass" for item in cap_evidence
    )
    return {
        "policy_schema_version": policy.get("schema_version"),
        "release": policy.get("release"),
        "status": policy.get("status"),
        "required_measurements": required_evidence,
        "hard_caps": cap_evidence,
        "passed": passed,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-artifact", type=pathlib.Path, required=True)
    parser.add_argument(
        "--profile",
        action="append",
        type=parse_profile,
        required=True,
        metavar="NAME=PATH",
    )
    parser.add_argument("--release-profile", default="z")
    parser.add_argument("--target", required=True)
    parser.add_argument("--policy", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--startup-samples", type=int, default=11)
    parser.add_argument("--pipe-bytes", type=int, default=8 * 1024 * 1024)
    parser.add_argument("--timeout", type=float, default=30.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    policy = json.loads(args.policy.read_bytes())
    profiles = dict(args.profile)
    if len(profiles) != len(args.profile):
        raise SystemExit("profile names must be unique")
    if args.release_profile not in profiles:
        raise SystemExit(f"release profile {args.release_profile!r} was not provided")
    if policy.get("selected_profile") != args.release_profile:
        raise SystemExit("release profile does not match the selected policy profile")

    release_artifact = artifact_metrics(args.release_artifact)
    comparisons = {
        name: artifact_metrics(path) for name, path in sorted(profiles.items())
    }
    measurement_errors = []
    if release_artifact["sha256"] != comparisons[args.release_profile]["sha256"]:
        measurement_errors.append("release artifact differs from the selected profile binary")

    runtime: dict[str, Any] = {}
    payload = pipe_payload(args.pipe_bytes)
    for name, operation in (
        (
            "startup",
            lambda: measure_startup(
                args.release_artifact, args.startup_samples, args.timeout
            ),
        ),
        (
            "pipe_throughput",
            lambda: measure_pipe(args.release_artifact, payload, args.timeout),
        ),
        (
            "peak_rss",
            lambda: measure_peak_rss(args.release_artifact, payload, args.timeout),
        ),
    ):
        try:
            runtime[name] = operation()
        except (OSError, RuntimeError, subprocess.TimeoutExpired, ValueError) as error:
            runtime[name] = unavailable(error)
            measurement_errors.append(f"{name}: {error}")

    report: dict[str, Any] = {
        "schema_version": 1,
        "target": args.target,
        "selected_profile": args.release_profile,
        "release_artifact": release_artifact,
        "profile_comparison": comparisons,
        "runtime": runtime,
    }
    try:
        report["budget_evidence"] = evaluate_policy(report, policy, args.target)
    except ValueError as error:
        report["budget_evidence"] = {"passed": False, "error": str(error)}
        measurement_errors.append(str(error))
    report["measurement_errors"] = measurement_errors
    report["passed"] = not measurement_errors and report["budget_evidence"]["passed"]

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        f"release metrics for {args.target}: "
        f"{release_artifact['uncompressed_bytes']} bytes; "
        f"budget {'passed' if report['passed'] else 'failed'}"
    )
    for error in measurement_errors:
        print(f"FAIL {error}", file=sys.stderr)
    for cap in report["budget_evidence"].get("hard_caps", []):
        if cap["result"] == "fail":
            print(
                f"FAIL {cap['metric']}: {cap['actual']} exceeds {cap['limit']}",
                file=sys.stderr,
            )
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
