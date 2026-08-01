#!/usr/bin/env python3
"""Create a flat, checksummed Tapas Actions artifact directory."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import stat


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--abi", required=True)
    parser.add_argument("--workflow-run", required=True)
    return parser.parse_args()


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    args = parse_args()
    if not args.binary.is_file():
        raise SystemExit(f"binary not found: {args.binary}")
    if args.output.exists():
        raise SystemExit(f"output already exists: {args.output}")
    if len(args.source_sha) != 40 or any(byte not in "0123456789abcdef" for byte in args.source_sha.lower()):
        raise SystemExit("--source-sha must be a full 40-character hexadecimal commit")

    args.output.mkdir(parents=True)
    binary = args.output / "tapas"
    shutil.copyfile(args.binary, binary)
    binary.chmod(binary.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    binary_digest = sha256(binary)

    metadata = {
        "schema_version": 1,
        "product": "tapas",
        "version": args.version,
        "source_sha": args.source_sha.lower(),
        "target": args.target,
        "abi": args.abi,
        "workflow_run": args.workflow_run,
        "binary": {
            "name": "tapas",
            "sha256": binary_digest,
            "uncompressed_bytes": binary.stat().st_size,
        },
    }
    metadata_path = args.output / "BUILD-METADATA.json"
    metadata_path.write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    checksums = args.output / "SHA256SUMS"
    checksums.write_text(
        f"{binary_digest}  tapas\n{sha256(metadata_path)}  BUILD-METADATA.json\n",
        encoding="ascii",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
