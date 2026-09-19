import contextlib
import importlib.util
import io
import json
import pathlib
import subprocess
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "smoke-supported-commands.py"


def load_module():
    spec = importlib.util.spec_from_file_location("smoke_supported_commands", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    if spec.loader is None:
        raise RuntimeError(f"cannot load {MODULE_PATH}")
    spec.loader.exec_module(module)
    return module


class SmokeSupportedCommandsTests(unittest.TestCase):
    def test_compact_result_requires_reduction_and_retained_facts(self) -> None:
        smoke = load_module()
        raw = smoke.Completed(0, b"building\nasset app.js\nbuilt in 10ms\n", b"")
        compact = smoke.Completed(0, b"asset app.js\nbuilt in 10ms\n", b"")

        result = smoke.evaluate("vite", raw, compact, facts=(b"app.js", b"built in"))

        self.assertEqual(result.status, smoke.Status.PASSED)
        self.assertLess(result.compact_bytes, result.raw_bytes)
        self.assertEqual(result.facts, ("app.js", "built in"))

    def test_compact_result_rejects_missing_facts_or_no_reduction(self) -> None:
        smoke = load_module()
        raw = smoke.Completed(0, b"important detail\n", b"")

        with self.assertRaisesRegex(smoke.VerificationError, "did not reduce"):
            smoke.evaluate("same", raw, raw, facts=(b"important",))
        with self.assertRaisesRegex(smoke.VerificationError, "did not reduce"):
            smoke.evaluate(
                "negligible",
                smoke.Completed(0, b"important detail\n", b""),
                smoke.Completed(0, b"important detai\n", b""),
                facts=(b"important",),
            )
        with self.assertRaisesRegex(smoke.VerificationError, "missing fact"):
            smoke.evaluate(
                "missing",
                raw,
                smoke.Completed(0, b"short\n", b""),
                facts=(b"important",),
            )

    def test_compact_result_accepts_generated_summary_cues(self) -> None:
        smoke = load_module()
        raw = smoke.Completed(0, b"match 1 with detail\nmatch 2 with detail\nmatch 3 with detail\n", b"")
        compact = smoke.Completed(0, b"match 1\n... 2 more matches\n", b"")

        result = smoke.evaluate(
            "grep",
            raw,
            compact,
            facts=(b"match 1",),
            summary_facts=(b"more matches",),
        )

        self.assertEqual(result.facts, ("match 1", "more matches"))

    def test_compact_result_can_use_a_route_specific_reduction_floor(self) -> None:
        smoke = load_module()
        raw = smoke.Completed(0, b"NAME: demo\nLAST DEPLOYED: today\nSTATUS: deployed\n", b"")
        compact = smoke.Completed(0, b"NAME: demo\nSTATUS: deployed\n", b"")

        result = smoke.evaluate(
            "helm-status",
            raw,
            compact,
            facts=(b"NAME: demo", b"STATUS: deployed"),
            minimum_reduction=0.05,
        )

        self.assertEqual(result.status, smoke.Status.PASSED)

    def test_exact_result_requires_identical_streams_and_exit_status(self) -> None:
        smoke = load_module()
        raw = smoke.Completed(2, b"partial\n", b"failure\n")

        result = smoke.evaluate("failure", raw, raw, exact=True, expect_failure=True)

        self.assertEqual(result.status, smoke.Status.PASSED)
        with self.assertRaisesRegex(smoke.VerificationError, "stdout changed"):
            smoke.evaluate(
                "failure",
                raw,
                smoke.Completed(2, b"different\n", b"failure\n"),
                exact=True,
                expect_failure=True,
            )

    def test_exact_result_requires_the_intended_outcome(self) -> None:
        smoke = load_module()
        success = smoke.Completed(0, b"[]\n", b"")
        failure = smoke.Completed(2, b"", b"unsupported option\n")

        with self.assertRaisesRegex(smoke.VerificationError, "unexpectedly failed"):
            smoke.evaluate("machine", failure, failure, exact=True)
        with self.assertRaisesRegex(smoke.VerificationError, "unexpectedly succeeded"):
            smoke.evaluate(
                "expected-failure",
                success,
                success,
                exact=True,
                expect_failure=True,
            )

    def test_required_skips_fail_the_summary(self) -> None:
        smoke = load_module()
        results = [
            smoke.Result("grep", smoke.Status.PASSED),
            smoke.Result("docker", smoke.Status.SKIPPED, detail="docker unavailable"),
        ]

        smoke.validate_summary(results, require_all=False)
        with self.assertRaisesRegex(smoke.VerificationError, "docker unavailable"):
            smoke.validate_summary(results, require_all=True)

    def test_cleanup_attempts_every_action_and_reports_failures(self) -> None:
        smoke = load_module()
        runner = object.__new__(smoke.Smoke)
        calls = []

        def fail(argv, cwd, *, timeout):
            calls.append((argv, cwd, timeout))
            return smoke.Completed(1, b"", b"still exists")

        runner.run = fail
        actions = [
            ("container", ["docker", "rm", "one"], pathlib.Path("/tmp"), 10),
            ("cluster", ["kind", "delete", "two"], pathlib.Path("/tmp"), 20),
        ]

        with self.assertRaisesRegex(smoke.VerificationError, "container.*cluster"):
            runner.cleanup(actions)

        self.assertEqual(len(calls), 2)

    @unittest.skipUnless(hasattr(Exception(), "add_note"), "requires exception notes")
    def test_case_failure_reports_cleanup_notes_in_text_and_json(self) -> None:
        smoke = load_module()

        def run(argv, **kwargs):
            if argv[:2] == ["docker", "info"]:
                return subprocess.CompletedProcess(argv, 0, b"1.0", b"")
            if argv[:3] == ["kind", "create", "cluster"]:
                return subprocess.CompletedProcess(argv, 1, b"", b"creation failed")
            if argv[:3] == ["kind", "delete", "cluster"]:
                return subprocess.CompletedProcess(argv, 1, b"", b"cluster still exists")
            raise AssertionError(f"unexpected command: {argv}")

        for output_format in ("text", "json"):
            with self.subTest(format=output_format):
                output = io.StringIO()
                arguments = [
                    str(MODULE_PATH), "--binary", str(MODULE_PATH),
                    "--case", "helm", "--format", output_format,
                ]
                with (
                    mock.patch("sys.argv", arguments),
                    mock.patch.object(smoke.shutil, "which", return_value="/synthetic/tool"),
                    mock.patch.object(smoke.subprocess, "run", side_effect=run),
                    contextlib.redirect_stdout(output),
                    contextlib.redirect_stderr(io.StringIO()),
                ):
                    status = smoke.main()
                self.assertEqual(status, 1)
                detail = (
                    json.loads(output.getvalue())[0]["detail"]
                    if output_format == "json" else output.getvalue()
                )
                self.assertIn("creation failed", detail)
                self.assertIn("cleanup failed: Kind cluster: cluster still exists", detail)


if __name__ == "__main__":
    unittest.main()
