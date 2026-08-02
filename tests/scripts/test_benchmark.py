from __future__ import annotations

import pathlib
import copy
import sys
import unittest
from contextlib import redirect_stderr
from io import StringIO


SCRIPTS = pathlib.Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))

import benchmark  # noqa: E402


class BenchmarkArgumentTests(unittest.TestCase):
    def test_baseline_comparison_requires_tapas_without_smll(self) -> None:
        args = benchmark.parse_args(["--baseline", "baseline.json", "--tapas-bin", "tapas"])

        self.assertEqual(args.mode, "compare-baseline")

        stderr = StringIO()
        with redirect_stderr(stderr), self.assertRaises(SystemExit):
            benchmark.parse_args(["--baseline", "baseline.json"])
        self.assertIn("--baseline requires --tapas-bin", stderr.getvalue())

        stderr = StringIO()
        with redirect_stderr(stderr), self.assertRaises(SystemExit):
            benchmark.parse_args(
                ["--baseline", "baseline.json", "--tapas-bin", "tapas", "--smll-bin", "smll"]
            )
        self.assertIn("--baseline cannot be combined with --smll-bin", stderr.getvalue())

    def test_write_baseline_requires_smll_and_rejects_tapas(self) -> None:
        args = benchmark.parse_args(["--write-baseline", "--smll-bin", "smll"])

        self.assertEqual(args.mode, "write-baseline")
        self.assertEqual(args.baseline, benchmark.DEFAULT_BASELINE)

        for argv, message in (
            (["--write-baseline"], "--write-baseline requires --smll-bin"),
            (
                ["--write-baseline", "--smll-bin", "smll", "--tapas-bin", "tapas"],
                "--write-baseline cannot be combined with --tapas-bin",
            ),
            (
                ["--write-baseline", "--smll-bin", "smll", "--case", "one"],
                "--write-baseline cannot be combined with --case",
            ),
        ):
            stderr = StringIO()
            with redirect_stderr(stderr), self.assertRaises(SystemExit):
                benchmark.parse_args(argv)
            self.assertIn(message, stderr.getvalue())

    def test_live_mode_preserves_smll_only_and_smll_vs_tapas_invocations(self) -> None:
        self.assertEqual(benchmark.parse_args(["--smll-bin", "smll"]).mode, "live")
        self.assertEqual(
            benchmark.parse_args(["--smll-bin", "smll", "--tapas-bin", "tapas"]).mode,
            "live",
        )


def metric(*, tokens: int | None = 5, exit_result: int = 0) -> dict[str, object]:
    return {
        "binary_sha256": "b" * 64,
        "stdout_sha256": "1" * 64,
        "stderr_sha256": "2" * 64,
        "stdout_bytes": 10,
        "stderr_bytes": 0,
        "stdout_proxy_tokens": tokens,
        "stderr_proxy_tokens": 0 if tokens is not None else None,
        "exit_result": exit_result,
        "elapsed_ns": 1,
        "assertion_errors": [],
    }


class BaselineComparisonTests(unittest.TestCase):
    def setUp(self) -> None:
        self.cases = [{"id": "one"}]
        self.metadata = {
            "corpus_sha256": "c" * 64,
            "source_commit": "source",
            "tokenizer": benchmark.TOKENIZER_METADATA,
        }
        self.baseline = {
            "schema_version": 2,
            **self.metadata,
            "smll_binary_sha256": "b" * 64,
            "records": [{"case_id": "one", "smll": metric()}],
        }

    def compare(self, baseline=None, cases=None, tapas=None):
        return benchmark.compare_baseline(
            baseline or self.baseline,
            cases or self.cases,
            tapas or {"one": metric()},
            **self.metadata,
        )

    def test_rejects_provenance_mismatches(self) -> None:
        for field, value in (
            ("corpus_sha256", "different"),
            ("source_commit", "different"),
            ("tokenizer", {**benchmark.TOKENIZER_METADATA, "asset_sha256": "different"}),
        ):
            baseline = copy.deepcopy(self.baseline)
            baseline[field] = value
            _, errors = self.compare(baseline=baseline)
            self.assertTrue(any(field in error for error in errors), errors)

        baseline = copy.deepcopy(self.baseline)
        baseline["records"][0]["smll"]["binary_sha256"] = "different"
        _, errors = self.compare(baseline=baseline)
        self.assertTrue(any("smll binary hash" in error for error in errors), errors)

    def test_rejects_missing_extra_and_duplicate_cases(self) -> None:
        duplicate_baseline = copy.deepcopy(self.baseline)
        duplicate_baseline["records"].append(copy.deepcopy(duplicate_baseline["records"][0]))
        _, errors = self.compare(baseline=duplicate_baseline)
        self.assertTrue(any("duplicate baseline case" in error for error in errors), errors)

        _, errors = self.compare(cases=[{"id": "one"}, {"id": "one"}])
        self.assertTrue(any("duplicate corpus case" in error for error in errors), errors)

        extra_baseline = copy.deepcopy(self.baseline)
        extra_baseline["records"].append({"case_id": "extra", "smll": metric()})
        _, errors = self.compare(baseline=extra_baseline)
        self.assertTrue(any("extra baseline cases: extra" in error for error in errors), errors)

        missing_baseline = copy.deepcopy(self.baseline)
        missing_baseline["records"] = []
        _, errors = self.compare(baseline=missing_baseline)
        self.assertTrue(any("missing baseline cases: one" in error for error in errors), errors)

    def test_targeted_comparison_validates_the_full_baseline(self) -> None:
        baseline = copy.deepcopy(self.baseline)
        baseline["records"].append({"case_id": "two", "smll": metric()})

        report, errors = benchmark.compare_baseline(
            baseline,
            self.cases,
            {"one": metric()},
            baseline_case_ids=["one", "two"],
            **self.metadata,
        )

        self.assertEqual(errors, [])
        self.assertEqual(report["cases"], 1)

        baseline["records"].pop()
        _, errors = benchmark.compare_baseline(
            baseline,
            self.cases,
            {"one": metric()},
            baseline_case_ids=["one", "two"],
            **self.metadata,
        )
        self.assertIn("missing baseline cases: two", errors)

    def test_hash_differences_and_aggregate_savings_are_report_only(self) -> None:
        tapas = metric(tokens=4)
        tapas["stdout_sha256"] = "9" * 64

        report, errors = self.compare(tapas={"one": tapas})

        self.assertEqual(errors, [])
        self.assertEqual(report["cases"], 1)
        self.assertEqual(report["exact_both_streams"], 0)
        self.assertEqual(report["smll_proxy_tokens"], 5)
        self.assertEqual(report["tapas_proxy_tokens"], 4)
        self.assertEqual(report["proxy_token_delta"], -1)
        self.assertFalse(report["records"][0]["stdout_exact"])

    def test_rejects_assertions_exits_unknown_tokens_and_per_case_regressions(self) -> None:
        scenarios = []

        tapas = metric()
        tapas["assertion_errors"] = ["missing fact"]
        scenarios.append((self.baseline, tapas, "assertion errors"))
        scenarios.append((self.baseline, metric(exit_result=7), "exit"))
        scenarios.append((self.baseline, metric(tokens=None), "token count is None"))
        scenarios.append((self.baseline, metric(tokens=6), "token regression"))

        baseline = copy.deepcopy(self.baseline)
        baseline["records"][0]["smll"] = metric(tokens=None)
        scenarios.append((baseline, metric(), "token count is None"))

        for baseline, tapas, message in scenarios:
            _, errors = self.compare(baseline=baseline, tapas={"one": tapas})
            self.assertTrue(any(message in error for error in errors), errors)


class BaselineWritingTests(unittest.TestCase):
    def test_baseline_binds_provenance_and_smll_measurements(self) -> None:
        baseline = benchmark.build_baseline(
            [{"id": "one", "expect": {"stdout": {"facts": []}, "stderr": {"facts": []}}}],
            {"one": metric()},
            corpus_sha256="c" * 64,
            source_commit="source",
            target="linux-x86_64",
            smll_binary_sha256="a" * 64,
        )

        self.assertEqual(baseline["corpus_sha256"], "c" * 64)
        self.assertEqual(baseline["source_commit"], "source")
        self.assertEqual(baseline["tokenizer"], benchmark.TOKENIZER_METADATA)
        self.assertEqual(baseline["smll_binary_sha256"], "a" * 64)
        self.assertEqual(baseline["records"][0]["smll"]["stdout_sha256"], "1" * 64)


if __name__ == "__main__":
    unittest.main()
