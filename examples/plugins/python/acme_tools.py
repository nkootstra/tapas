#!/usr/bin/env python3
"""Dependency-free Tapas filter example for Acme test output."""

import base64
import json
import sys


def decode(value):
    return base64.b64decode(value)


def encode(value):
    return base64.b64encode(value).decode("ascii")


print(json.dumps({"protocol": "tapas-filter", "versions": [1]}), flush=True)
request = json.loads(sys.stdin.readline())
argv = [decode(argument) for argument in request["argv_b64"]]
command = argv[0].replace(b"\\", b"/").rsplit(b"/", 1)[-1] if argv else b""
shape = argv[1] if command == b"acme" and len(argv) > 1 else b""
if shape not in (b"test", b"build"):
    print(json.dumps({"version": 1, "result": "decline"}), flush=True)
    sys.exit(0)

stdout = decode(request["stdout_b64"])
stderr = decode(request["stderr_b64"])

stdout_lines = stdout.splitlines(keepends=True)
prefix = b"PASS " if shape == b"test" else b"COMPILE "
count = sum(line.startswith(prefix) for line in stdout_lines)
unit = b"cases" if shape == b"test" else b"targets"
compact_stdout = prefix.rstrip() + f" {count} ".encode() + unit + b"\n" + b"".join(
    line for line in stdout_lines if not line.startswith(prefix)
)

stderr_lines = stderr.splitlines(keepends=True)
warnings = [line for line in stderr_lines if line.startswith(b"WARN ")]
compact_stderr = b""
if warnings:
    compact_stderr += warnings[0].rstrip(b"\r\n")
    compact_stderr += f" (repeated {len(warnings)} times)\n".encode()
compact_stderr += b"".join(line for line in stderr_lines if not line.startswith(b"WARN "))

response = {
    "version": 1,
    "result": "transform",
    "evidence": "fact-complete",
    "stdout_b64": encode(compact_stdout),
    "stderr_b64": encode(compact_stderr),
}
print(json.dumps(response), flush=True)
