#!/usr/bin/env python3
"""Run only wrapper-mode supported-command cases from the frozen contract."""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import tempfile


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--tool", choices=("smll", "tapas"), required=True)
    parser.add_argument("--cases", type=pathlib.Path, default=pathlib.Path("tests/compat/smll-v1.9.0/cases.json"))
    parser.add_argument("--contract", type=pathlib.Path, default=pathlib.Path("tests/compat/smll-v1.9.0"))
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    document = json.loads(args.cases.read_text(encoding="utf-8"))
    selected = {"schema_version": document["schema_version"], "source_commit": document["source_commit"], "cases": [case for case in document["cases"] if case["mode"] == "wrapper"]}
    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", encoding="utf-8") as handle:
        json.dump(selected, handle)
        handle.flush()
        return subprocess.call([sys.executable, str(pathlib.Path(__file__).with_name("parity.py")), "--binary", str(args.binary), "--tool", args.tool, "--cases", handle.name, "--contract", str(args.contract)])


if __name__ == "__main__":
    raise SystemExit(main())
