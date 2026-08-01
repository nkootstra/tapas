#!/usr/bin/env python3
"""Run the fixed compatibility corpus with the vendored o200k_base proxy."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import platform
import sys
import time
from typing import Any

import parity


TOKENIZER_HASH = "446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d"
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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--smll-bin", type=pathlib.Path, required=True)
    parser.add_argument("--tapas-bin", type=pathlib.Path)
    parser.add_argument("--cases", type=pathlib.Path, default=parity.DEFAULT_CASES)
    parser.add_argument("--contract", type=pathlib.Path, default=parity.DEFAULT_CONTRACT)
    parser.add_argument("--tokenizer", type=pathlib.Path, default=pathlib.Path("tests/tokenizers/o200k_base.tiktoken"))
    parser.add_argument("--case", action="append", dest="case_ids")
    parser.add_argument("--timeout", type=float, default=10.0)
    parser.add_argument("--output", type=pathlib.Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        encoder = load_encoder(args.tokenizer)
    except (OSError, ValueError, RuntimeError) as exc:
        print(f"benchmark setup failed: {exc}", file=sys.stderr)
        return 2
    raw_cases = args.cases.read_bytes()
    document = json.loads(raw_cases)
    cases = document["cases"]
    if args.case_ids:
        requested = set(args.case_ids)
        cases = [case for case in cases if case["id"] in requested]
        missing = requested - {case["id"] for case in cases}
        if missing:
            print(f"unknown cases: {', '.join(sorted(missing))}", file=sys.stderr)
            return 2
    records = []
    failed = False
    target = f"{platform.system().lower()}-{platform.machine().lower()}"
    corpus_sha256 = digest(raw_cases)
    smll_sha256 = binary_hash(args.smll_bin)
    tapas_sha256 = binary_hash(args.tapas_bin) if args.tapas_bin else None
    for case in cases:
        record: dict[str, Any] = {
            "case_id": case["id"],
            "source_commit": document["source_commit"],
            "target": target,
            "corpus_sha256": corpus_sha256,
            "asserted_facts": {stream: case["expect"][stream]["facts"] for stream in ("stdout", "stderr")},
            "tokenizer": {"package": "tiktoken", "version": "0.12.0", "encoding": "o200k_base", "asset_sha256": TOKENIZER_HASH},
        }
        smll = run_tool(args.smll_bin, smll_sha256, "smll", case, args.contract, args.timeout, encoder)
        record["smll"] = smll
        failed |= bool(smll["assertion_errors"])
        if args.tapas_bin:
            tapas = run_tool(args.tapas_bin, tapas_sha256, "tapas", case, args.contract, args.timeout, encoder)
            record["tapas"] = tapas
            failed |= bool(tapas["assertion_errors"])
            smll_tokens = None if smll["stdout_proxy_tokens"] is None or smll["stderr_proxy_tokens"] is None else smll["stdout_proxy_tokens"] + smll["stderr_proxy_tokens"]
            tapas_tokens = None if tapas["stdout_proxy_tokens"] is None or tapas["stderr_proxy_tokens"] is None else tapas["stdout_proxy_tokens"] + tapas["stderr_proxy_tokens"]
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
