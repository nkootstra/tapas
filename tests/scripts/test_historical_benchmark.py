from __future__ import annotations

import pathlib
import sys
import unittest


SCRIPTS = pathlib.Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))

import historical_benchmark  # noqa: E402


class IntentionalDifferenceTests(unittest.TestCase):
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
