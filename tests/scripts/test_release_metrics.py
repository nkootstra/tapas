from __future__ import annotations

import pathlib
import sys
import tempfile
import unittest


SCRIPTS = pathlib.Path(__file__).resolve().parents[2] / "scripts"
sys.path.insert(0, str(SCRIPTS))

import release_metrics  # noqa: E402


class ArtifactMetricsTests(unittest.TestCase):
    def test_artifact_metrics_are_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as temp_name:
            binary = pathlib.Path(temp_name) / "tapas"
            binary.write_bytes(b"tapas\0" * 100)

            first = release_metrics.artifact_metrics(binary)
            second = release_metrics.artifact_metrics(binary)

        self.assertEqual(first, second)
        self.assertEqual(first["uncompressed_bytes"], 600)
        self.assertEqual(len(first["sha256"]), 64)
        self.assertGreater(first["gzip_bytes"], 0)


class RssParsingTests(unittest.TestCase):
    def test_parses_linux_time_output_as_kibibytes(self) -> None:
        stderr = b"TAPAS_MAX_RSS_KIB=1234\n"

        self.assertEqual(release_metrics.parse_linux_rss(stderr), 1234 * 1024)

    def test_parses_macos_time_output_as_bytes(self) -> None:
        stderr = b"       567890  maximum resident set size\n"

        self.assertEqual(release_metrics.parse_macos_rss(stderr), 567890)

    def test_missing_rss_marker_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "maximum resident set size"):
            release_metrics.parse_macos_rss(b"no measurement here\n")


class BudgetEvaluationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.policy = {
            "schema_version": 1,
            "release": "0.1.0",
            "status": "internal-baseline",
            "selected_profile": "z",
            "required_measurements": [
                "release_artifact.uncompressed_bytes",
                "runtime.peak_rss.bytes",
            ],
            "targets": {
                "test-target": {
                    "hard_caps": {
                        "release_artifact.uncompressed_bytes": 100,
                    }
                }
            },
        }

    def test_passes_when_required_evidence_exists_and_caps_hold(self) -> None:
        report = {
            "release_artifact": {"uncompressed_bytes": 99},
            "runtime": {"peak_rss": {"status": "measured", "bytes": 2048}},
        }

        evidence = release_metrics.evaluate_policy(report, self.policy, "test-target")

        self.assertTrue(evidence["passed"])
        self.assertEqual(evidence["hard_caps"][0]["result"], "pass")
        self.assertTrue(
            all(item["available"] for item in evidence["required_measurements"])
        )

    def test_fails_when_a_cap_is_exceeded(self) -> None:
        report = {
            "release_artifact": {"uncompressed_bytes": 101},
            "runtime": {"peak_rss": {"status": "measured", "bytes": 2048}},
        }

        evidence = release_metrics.evaluate_policy(report, self.policy, "test-target")

        self.assertFalse(evidence["passed"])
        self.assertEqual(evidence["hard_caps"][0]["result"], "fail")

    def test_fails_closed_when_required_rss_is_unavailable(self) -> None:
        report = {
            "release_artifact": {"uncompressed_bytes": 99},
            "runtime": {"peak_rss": {"status": "unavailable", "reason": "unsupported"}},
        }

        evidence = release_metrics.evaluate_policy(report, self.policy, "test-target")

        self.assertFalse(evidence["passed"])
        missing = [
            item for item in evidence["required_measurements"] if not item["available"]
        ]
        self.assertEqual(
            [item["metric"] for item in missing], ["runtime.peak_rss.bytes"]
        )

    def test_rejects_an_unknown_target(self) -> None:
        with self.assertRaisesRegex(ValueError, "no release budget for target"):
            release_metrics.evaluate_policy({}, self.policy, "other-target")


if __name__ == "__main__":
    unittest.main()
