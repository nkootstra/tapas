#!/usr/bin/env python3
"""Run the fixed compatibility corpus with the vendored o200k_base proxy."""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import pathlib
import platform
import sys
import time
from typing import Any

import parity


TOKENIZER_HASH = "446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d"
DEFAULT_BASELINE = pathlib.Path(
    "tests/compat/smll-v1.9.0/executable-benchmark-baseline.json"
)
TOKENIZER_METADATA = {
    "package": "tiktoken",
    "version": "0.12.0",
    "encoding": "o200k_base",
    "asset_sha256": TOKENIZER_HASH,
}
PATTERN = "|".join(
    (
        r"[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]*[\p{Ll}\p{Lm}\p{Lo}\p{M}]+(?i:'s|'t|'re|'ve|'m|'ll|'d)?",
        r"[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]+[\p{Ll}\p{Lm}\p{Lo}\p{M}]*(?i:'s|'t|'re|'ve|'m|'ll|'d)?",
        r"\p{N}{1,3}",
        r" ?[^\s\p{L}\p{N}]+[\r\n/]*",
        r"\s*[\r\n]+",
        r"\s+(?!\S)",
        r"\s+",
    )
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def binary_hash(path: pathlib.Path) -> str:
    return digest(path.read_bytes())


def load_encoder(asset: pathlib.Path):
    data = asset.read_bytes()
    if digest(data) != TOKENIZER_HASH:
        raise ValueError(f"o200k_base asset hash mismatch: {digest(data)}")
    try:
        import tiktoken
        from tiktoken.load import load_tiktoken_bpe
    except ImportError as exc:
        raise RuntimeError("install scripts/requirements-benchmark.txt") from exc
    ranks = load_tiktoken_bpe(str(asset), expected_hash=TOKENIZER_HASH)
    return tiktoken.Encoding(
        "o200k_base_vendored",
        pat_str=PATTERN,
        mergeable_ranks=ranks,
        special_tokens={"<|endoftext|>": 199999, "<|endofprompt|>": 200018},
    )


def tokens(encoder: Any, data: bytes) -> int | None:
    try:
        value = data.decode("utf-8")
    except UnicodeDecodeError:
        return None
    return len(encoder.encode(value))


def run_tool(
    binary: pathlib.Path,
    binary_sha256: str,
    tool: str,
    case: dict[str, Any],
    contract: pathlib.Path,
    timeout: float,
    encoder: Any,
) -> dict[str, Any]:
    started = time.perf_counter_ns()
    result = parity.run_case(binary.resolve(), case, contract, timeout, tool)
    elapsed = time.perf_counter_ns() - started
    return {
        "binary_sha256": binary_sha256,
        "stdout_sha256": digest(result.stdout),
        "stderr_sha256": digest(result.stderr),
        "stdout_bytes": len(result.stdout),
        "stderr_bytes": len(result.stderr),
        "stdout_proxy_tokens": tokens(encoder, result.stdout),
        "stderr_proxy_tokens": tokens(encoder, result.stderr),
        "exit_result": result.returncode,
        "elapsed_ns": elapsed,
        "assertion_errors": result.errors,
    }


def compare_baseline(
    baseline: dict[str, Any],
    cases: list[dict[str, Any]],
    tapas_by_id: dict[str, dict[str, Any]],
    *,
    baseline_case_ids: list[str] | None = None,
    corpus_sha256: str,
    source_commit: str,
    tokenizer: dict[str, str],
) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    for field, expected in (
        ("corpus_sha256", corpus_sha256),
        ("source_commit", source_commit),
        ("tokenizer", tokenizer),
    ):
        if baseline.get(field) != expected:
            errors.append(f"baseline {field} mismatch")
    case_ids = [case.get("id") for case in cases]
    expected_baseline_ids = case_ids if baseline_case_ids is None else baseline_case_ids
    baseline_ids = [record.get("case_id") for record in baseline.get("records", [])]
    duplicate_cases = sorted(
        case_id
        for case_id, count in collections.Counter(expected_baseline_ids).items()
        if count > 1
    )
    duplicate_baseline = sorted(
        case_id
        for case_id, count in collections.Counter(baseline_ids).items()
        if count > 1
    )
    if duplicate_cases:
        errors.append(f"duplicate corpus cases: {', '.join(duplicate_cases)}")
    if duplicate_baseline:
        errors.append(f"duplicate baseline cases: {', '.join(duplicate_baseline)}")
    case_set = set(case_ids)
    expected_baseline_set = set(expected_baseline_ids)
    baseline_set = set(baseline_ids)
    if missing := sorted(expected_baseline_set - baseline_set):
        errors.append(f"missing baseline cases: {', '.join(missing)}")
    if extra := sorted(baseline_set - expected_baseline_set):
        errors.append(f"extra baseline cases: {', '.join(extra)}")
    tapas_set = set(tapas_by_id)
    if missing := sorted(case_set - tapas_set):
        errors.append(f"missing Tapas results: {', '.join(missing)}")
    if extra := sorted(tapas_set - case_set):
        errors.append(f"extra Tapas results: {', '.join(extra)}")

    baseline_by_id = {
        record["case_id"]: record for record in baseline.get("records", [])
    }
    smll_binary_sha256 = baseline.get("smll_binary_sha256")
    if not isinstance(smll_binary_sha256, str) or len(smll_binary_sha256) != 64:
        errors.append("baseline smll binary hash is missing or invalid")
    for case_id, record in baseline_by_id.items():
        if record.get("smll", {}).get("binary_sha256") != smll_binary_sha256:
            errors.append(f"{case_id}: smll binary hash differs from baseline metadata")
    records = []
    smll_total = 0
    tapas_total = 0
    exact_both = 0
    for case_id in case_ids:
        if case_id not in baseline_by_id or case_id not in tapas_by_id:
            continue
        smll = baseline_by_id[case_id].get("smll", {})
        tapas = tapas_by_id[case_id]
        if smll.get("assertion_errors"):
            errors.append(f"{case_id}: baseline assertion errors: {'; '.join(smll['assertion_errors'])}")
        if tapas.get("assertion_errors"):
            errors.append(f"{case_id}: Tapas assertion errors: {'; '.join(tapas['assertion_errors'])}")
        if tapas.get("exit_result") != smll.get("exit_result"):
            errors.append(
                f"{case_id}: exit {tapas.get('exit_result')} != smll {smll.get('exit_result')}"
            )
        smll_tokens = _total_tokens(smll)
        tapas_tokens = _total_tokens(tapas)
        delta = None
        if smll_tokens is None or tapas_tokens is None:
            errors.append(f"{case_id}: token count is None")
        else:
            delta = tapas_tokens - smll_tokens
            smll_total += smll_tokens
            tapas_total += tapas_tokens
            if delta > 0:
                errors.append(
                    f"{case_id}: token regression {tapas_tokens} > {smll_tokens}"
                )
        stdout_exact = tapas.get("stdout_sha256") == smll.get("stdout_sha256")
        stderr_exact = tapas.get("stderr_sha256") == smll.get("stderr_sha256")
        exact_both += int(stdout_exact and stderr_exact)
        records.append(
            {
                "case_id": case_id,
                "smll": smll,
                "tapas": tapas,
                "stdout_exact": stdout_exact,
                "stderr_exact": stderr_exact,
                "proxy_token_delta": delta,
            }
        )
    savings = (
        0.0
        if smll_total == 0
        else round((smll_total - tapas_total) * 100 / smll_total, 2)
    )
    report = {
        "schema_version": 2,
        "cases": len(case_ids),
        "exact_both_streams": exact_both,
        "smll_proxy_tokens": smll_total,
        "tapas_proxy_tokens": tapas_total,
        "proxy_token_delta": tapas_total - smll_total,
        "proxy_token_savings_percent": savings,
        "records": records,
    }
    return report, errors


def _total_tokens(metrics: dict[str, Any]) -> int | None:
    stdout = metrics.get("stdout_proxy_tokens")
    stderr = metrics.get("stderr_proxy_tokens")
    if stdout is None or stderr is None:
        return None
    return int(stdout) + int(stderr)


def build_baseline(
    cases: list[dict[str, Any]],
    smll_by_id: dict[str, dict[str, Any]],
    *,
    corpus_sha256: str,
    source_commit: str,
    target: str,
    smll_binary_sha256: str,
) -> dict[str, Any]:
    return {
        "schema_version": 2,
        "source_commit": source_commit,
        "target": target,
        "corpus_sha256": corpus_sha256,
        "tokenizer": TOKENIZER_METADATA,
        "smll_binary_sha256": smll_binary_sha256,
        "records": [
            {
                "case_id": case["id"],
                "asserted_facts": {
                    stream: case["expect"][stream]["facts"]
                    for stream in ("stdout", "stderr")
                },
                "smll": smll_by_id[case["id"]],
            }
            for case in cases
        ],
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--smll-bin", type=pathlib.Path)
    parser.add_argument("--tapas-bin", type=pathlib.Path)
    parser.add_argument("--baseline", type=pathlib.Path)
    parser.add_argument("--write-baseline", action="store_true")
    parser.add_argument("--cases", type=pathlib.Path, default=parity.DEFAULT_CASES)
    parser.add_argument("--contract", type=pathlib.Path, default=parity.DEFAULT_CONTRACT)
    parser.add_argument("--tokenizer", type=pathlib.Path, default=pathlib.Path("tests/tokenizers/o200k_base.tiktoken"))
    parser.add_argument("--case", action="append", dest="case_ids")
    parser.add_argument("--timeout", type=float, default=10.0)
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args(argv)
    if args.write_baseline:
        if args.smll_bin is None:
            parser.error("--write-baseline requires --smll-bin")
        if args.tapas_bin is not None:
            parser.error("--write-baseline cannot be combined with --tapas-bin")
        if args.case_ids:
            parser.error("--write-baseline cannot be combined with --case")
        if args.baseline is None:
            args.baseline = DEFAULT_BASELINE
        args.mode = "write-baseline"
    elif args.baseline is not None:
        if args.tapas_bin is None:
            parser.error("--baseline requires --tapas-bin")
        if args.smll_bin is not None:
            parser.error("--baseline cannot be combined with --smll-bin")
        args.mode = "compare-baseline"
    else:
        if args.smll_bin is None:
            parser.error("live benchmark requires --smll-bin")
        args.mode = "live"
    return args


def main() -> int:
    args = parse_args()
    try:
        encoder = load_encoder(args.tokenizer)
    except (OSError, ValueError, RuntimeError) as exc:
        print(f"benchmark setup failed: {exc}", file=sys.stderr)
        return 2
    raw_cases = args.cases.read_bytes()
    document = json.loads(raw_cases)
    all_cases = document["cases"]
    cases = all_cases
    if args.case_ids:
        requested = set(args.case_ids)
        cases = [case for case in cases if case["id"] in requested]
        missing = requested - {case["id"] for case in cases}
        if missing:
            print(f"unknown cases: {', '.join(sorted(missing))}", file=sys.stderr)
            return 2
    target = f"{platform.system().lower()}-{platform.machine().lower()}"
    corpus_sha256 = digest(raw_cases)
    source_commit = document["source_commit"]

    if args.mode == "write-baseline":
        try:
            smll_sha256 = binary_hash(args.smll_bin)
        except OSError as exc:
            print(f"benchmark setup failed: {exc}", file=sys.stderr)
            return 2
        smll_by_id = {
            case["id"]: run_tool(
                args.smll_bin,
                smll_sha256,
                "smll",
                case,
                args.contract,
                args.timeout,
                encoder,
            )
            for case in cases
        }
        invalid = [
            case_id
            for case_id, metrics in smll_by_id.items()
            if metrics["assertion_errors"] or _total_tokens(metrics) is None
        ]
        if invalid:
            print(
                "refusing to write invalid smll baseline: " + ", ".join(invalid),
                file=sys.stderr,
            )
            return 1
        baseline = build_baseline(
            cases,
            smll_by_id,
            corpus_sha256=corpus_sha256,
            source_commit=source_commit,
            target=target,
            smll_binary_sha256=smll_sha256,
        )
        args.baseline.parent.mkdir(parents=True, exist_ok=True)
        args.baseline.write_text(
            json.dumps(baseline, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        print(f"wrote {args.baseline} with {len(cases)} pinned smll cases")
        return 0

    if args.mode == "compare-baseline":
        try:
            baseline = json.loads(args.baseline.read_bytes())
            tapas_sha256 = binary_hash(args.tapas_bin)
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            print(f"benchmark setup failed: {exc}", file=sys.stderr)
            return 2
        tapas_by_id = {
            case["id"]: run_tool(
                args.tapas_bin,
                tapas_sha256,
                "tapas",
                case,
                args.contract,
                args.timeout,
                encoder,
            )
            for case in cases
        }
        report, errors = compare_baseline(
            baseline,
            cases,
            tapas_by_id,
            baseline_case_ids=[case["id"] for case in all_cases],
            corpus_sha256=corpus_sha256,
            source_commit=source_commit,
            tokenizer=TOKENIZER_METADATA,
        )
        report.update(
            {
                "source_commit": source_commit,
                "target": target,
                "corpus_sha256": corpus_sha256,
                "tokenizer": TOKENIZER_METADATA,
                "smll_binary_sha256": baseline.get("smll_binary_sha256"),
                "tapas_binary_sha256": tapas_sha256,
            }
        )
        rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
        else:
            print(rendered, end="")
        print(
            f"executable benchmark: {report['exact_both_streams']}/{report['cases']} "
            f"exact; tokens {report['smll_proxy_tokens']} -> "
            f"{report['tapas_proxy_tokens']} ({report['proxy_token_delta']:+d})",
            file=sys.stderr if args.output else sys.stdout,
        )
        for error in errors:
            print(f"FAIL {error}", file=sys.stderr)
        return int(bool(errors))

    records = []
    failed = False
    smll_sha256 = binary_hash(args.smll_bin)
    tapas_sha256 = binary_hash(args.tapas_bin) if args.tapas_bin else None
    for case in cases:
        record: dict[str, Any] = {
            "case_id": case["id"],
            "source_commit": source_commit,
            "target": target,
            "corpus_sha256": corpus_sha256,
            "asserted_facts": {stream: case["expect"][stream]["facts"] for stream in ("stdout", "stderr")},
            "tokenizer": TOKENIZER_METADATA,
        }
        smll = run_tool(args.smll_bin, smll_sha256, "smll", case, args.contract, args.timeout, encoder)
        record["smll"] = smll
        failed |= bool(smll["assertion_errors"])
        if args.tapas_bin:
            tapas = run_tool(args.tapas_bin, tapas_sha256, "tapas", case, args.contract, args.timeout, encoder)
            record["tapas"] = tapas
            failed |= bool(tapas["assertion_errors"])
            smll_tokens = _total_tokens(smll)
            tapas_tokens = _total_tokens(tapas)
            record["proxy_token_delta"] = None if smll_tokens is None or tapas_tokens is None else tapas_tokens - smll_tokens
            if record["proxy_token_delta"] is not None and record["proxy_token_delta"] > 0:
                failed = True
        records.append(record)
    report = {"schema_version": 1, "records": records}
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
