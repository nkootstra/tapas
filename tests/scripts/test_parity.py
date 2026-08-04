from __future__ import annotations

import pathlib
import sys
import tempfile
import unittest


SCRIPTS = pathlib.Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))

import parity  # noqa: E402


class ParityFixtureTests(unittest.TestCase):
    def test_missing_fixtures_are_reported_before_execution(self) -> None:
        case = {
            "stdin": {"fixture": "fixtures/input.txt"},
            "child": {"stdout": {"fixture": "fixtures/output.txt"}},
        }
        with tempfile.TemporaryDirectory() as directory:
            contract = pathlib.Path(directory)
            (contract / "fixtures").mkdir()
            (contract / "fixtures/input.txt").write_bytes(b"input")
            self.assertEqual(
                parity.missing_fixtures([case], contract),
                ["fixtures/output.txt"],
            )

    def test_fixture_paths_walk_nested_case_values(self) -> None:
        self.assertEqual(
            sorted(
                parity.fixture_paths(
                    {"a": {"fixture": "one"}, "b": [{"fixture": "two"}]}
                )
            ),
            ["one", "two"],
        )


if __name__ == "__main__":
    unittest.main()
