from __future__ import annotations

import pathlib
import re
import unittest


REQUIREMENTS = (
    pathlib.Path(__file__).resolve().parents[2] / "scripts" / "requirements-benchmark.txt"
)
EXPECTED_CLOSURE = {
    "certifi",
    "charset-normalizer",
    "idna",
    "regex",
    "requests",
    "tiktoken",
    "urllib3",
}


def locked_requirements() -> dict[str, list[str]]:
    logical_lines: list[str] = []
    pending = ""
    for raw_line in REQUIREMENTS.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        pending += line.removesuffix("\\").strip() + " "
        if not line.endswith("\\"):
            logical_lines.append(pending.strip())
            pending = ""
    if pending:
        raise AssertionError("benchmark requirements end in an incomplete continuation")

    result = {}
    for line in logical_lines:
        requirement, *options = line.split()
        name, separator, version = requirement.partition("==")
        if not separator or not version:
            raise AssertionError(f"benchmark dependency is not exactly pinned: {line}")
        if name in result:
            raise AssertionError(f"duplicate benchmark dependency: {name}")
        result[name] = options
    return result


class BenchmarkLockTests(unittest.TestCase):
    def test_complete_dependency_closure_is_exactly_pinned_and_hashed(self) -> None:
        requirements = locked_requirements()

        self.assertEqual(set(requirements), EXPECTED_CLOSURE)
        for name, options in requirements.items():
            self.assertTrue(options, f"{name} has no artifact hash")
            for option in options:
                self.assertRegex(option, re.compile(r"^--hash=sha256:[0-9a-f]{64}$"))


if __name__ == "__main__":
    unittest.main()
