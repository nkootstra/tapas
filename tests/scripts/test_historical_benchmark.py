from __future__ import annotations

import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


SCRIPTS = pathlib.Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))

import historical_benchmark  # noqa: E402


class IntentionalDifferenceTests(unittest.TestCase):
    def test_compare_rejects_stream_changes_with_the_same_combined_output(self) -> None:
        class Encoder:
            @staticmethod
            def encode(value: str) -> list[str]:
                return list(value)

        corpus = b"pinned corpus"
        case = {
            "name": "intentional",
            "category": "listing",
            "fixture": "case.txt",
            "command": ["find", "."],
        }
        baseline = {
            "corpus_sha256": historical_benchmark.benchmark.digest(corpus),
            "tokenizer_sha256": historical_benchmark.benchmark.TOKENIZER_HASH,
            "records": [
                {
                    "name": "intentional",
                    "exit_result": 0,
                    "combined_sha256": historical_benchmark.benchmark.digest(b"ab"),
                    "proxy_tokens": len("ab"),
                    "visible_facts": [],
                }
            ],
        }
        completed = subprocess.CompletedProcess(
            args=["tapas"], returncode=0, stdout=b"", stderr=b"ab"
        )

        with tempfile.TemporaryDirectory() as temp_name:
            contract = pathlib.Path(temp_name)
            (contract / "fixtures").mkdir()
            (contract / "fixtures/case.txt").write_bytes(b"raw")
            with mock.patch.object(
                historical_benchmark, "run_case", return_value=completed
            ):
                _, errors = historical_benchmark.compare(
                    pathlib.Path("tapas"),
                    [case],
                    corpus,
                    baseline,
                    contract,
                    Encoder(),
                    1.0,
                    {"intentional": 0},
                    {"intentional": historical_benchmark.benchmark.digest(b"ab")},
                    {
                        "intentional": (
                            historical_benchmark.benchmark.digest(b"a"),
                            historical_benchmark.benchmark.digest(b"b"),
                        )
                    },
                )

        self.assertIn(
            "intentional: streams do not match pinned intentional output", errors
        )

    def test_intentional_outputs_are_pinned_per_case(self) -> None:
        document = {
            "differences": [
                {
                    "case": "find-plain-many",
                    "expected_combined_sha256": "a" * 64,
                    "expected_stdout_sha256": "b" * 64,
                    "expected_stderr_sha256": "c" * 64,
                }
            ]
        }

        self.assertEqual(
            historical_benchmark.intentional_difference_hashes(document),
            {"find-plain-many": "a" * 64},
        )
        self.assertEqual(
            historical_benchmark.intentional_difference_stream_hashes(document),
            {"find-plain-many": ("b" * 64, "c" * 64)},
        )
        with self.assertRaises(ValueError):
            historical_benchmark.intentional_difference_hashes(
                {"differences": [{"case": "find-plain-many"}]}
            )
        with self.assertRaises(ValueError):
            historical_benchmark.intentional_difference_hashes(
                {
                    "differences": [
                        {
                            "case": "find-plain-many",
                            "expected_combined_sha256": "not-a-sha256",
                        }
                    ]
                }
            )
        with self.assertRaises(ValueError):
            historical_benchmark.intentional_difference_stream_hashes(
                {
                    "differences": [
                        {
                            "case": "find-plain-many",
                            "expected_stdout_sha256": "b" * 64,
                        }
                    ]
                }
            )

    def test_intentional_output_must_match_its_pinned_hash(self) -> None:
        hashes = {"find-plain-many": "a" * 64}

        self.assertIsNone(
            historical_benchmark.intentional_output_error(
                "find-plain-many", "a" * 64, hashes
            )
        )
        self.assertIn(
            "pinned intentional output",
            historical_benchmark.intentional_output_error(
                "find-plain-many", "b" * 64, hashes
            ),
        )
        self.assertIsNone(
            historical_benchmark.intentional_output_error("other", "b" * 64, hashes)
        )

    def test_missing_token_limit_defaults_to_zero(self) -> None:
        allowances = historical_benchmark.intentional_difference_allowances(
            {"differences": [{"case": "find-plain-many"}]}
        )

        self.assertEqual(allowances, {"find-plain-many": 0})
        self.assertIn(
            "allowed 0",
            historical_benchmark.token_regression_error(
                "find-plain-many", 108, 107, allowances
            ),
        )

    def test_token_increase_is_bounded_per_case(self) -> None:
        allowances = {"find-plain-many": 18}

        self.assertIsNone(
            historical_benchmark.token_regression_error(
                "find-plain-many", 125, 107, allowances
            )
        )
        self.assertIn(
            "allowed 18",
            historical_benchmark.token_regression_error(
                "find-plain-many", 126, 107, allowances
            ),
        )
        self.assertIn(
            "allowed 0",
            historical_benchmark.token_regression_error("other", 108, 107, allowances),
        )

    def test_allowance_document_rejects_invalid_or_duplicate_limits(self) -> None:
        for value in (-1, 1.5, True):
            with self.subTest(value=value), self.assertRaises(ValueError):
                historical_benchmark.intentional_difference_allowances(
                    {
                        "differences": [
                            {"case": "find-plain-many", "max_proxy_token_increase": value}
                        ]
                    }
                )

        with self.assertRaises(ValueError):
            historical_benchmark.intentional_difference_allowances(
                {"differences": [{"case": "same"}, {"case": "same"}]}
            )


if __name__ == "__main__":
    unittest.main()
