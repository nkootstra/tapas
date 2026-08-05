from __future__ import annotations

import pathlib
import subprocess
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


class ParityBaselineComparisonTests(unittest.TestCase):
    def test_baseline_tool_uses_smll_environment_mapping(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = root / "smll"
            baseline.write_text(
                "#!/bin/sh\n"
                "test \"${SMLL_PARITY_TOOL_MAPPING-}\" = baseline-tool-selected || exit 21\n"
                "test -z \"${TAPAS_PARITY_TOOL_MAPPING-}\" || exit 22\n"
                "printf 'mapped environment\\n'\n",
                encoding="utf-8",
            )
            baseline.chmod(0o755)
            candidate = root / "tapas"
            candidate.write_text(
                "#!/bin/sh\n"
                "test \"${TAPAS_PARITY_TOOL_MAPPING-}\" = baseline-tool-selected || exit 23\n"
                "test -z \"${SMLL_PARITY_TOOL_MAPPING-}\" || exit 24\n"
                "printf 'mapped environment\\n'\n",
                encoding="utf-8",
            )
            candidate.chmod(0o755)
            cases = root / "cases.json"
            cases.write_text(
                """{
  "cases": [{
    "id": "comparison:baseline-tool-environment",
    "oracle": "smll",
    "mode": "pipe",
    "argv": ["unused"],
    "stdin": {"base64": ""},
    "env": {
      "set": {"SMLL_PARITY_TOOL_MAPPING": "baseline-tool-selected"},
      "unset": []
    },
    "expect": {
      "termination": {"exit_code": 0, "signal": null},
      "stdout": {"facts": ["mapped environment"], "byte_exact": false},
      "stderr": {"facts": [], "byte_exact": false},
      "incomplete_output": {"diagnostic_facts": []}
    }
  }]
}
""",
                encoding="utf-8",
            )

            proc = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "parity.py"),
                    "--binary",
                    str(candidate),
                    "--baseline-binary",
                    str(baseline),
                    "--tool",
                    "tapas",
                    "--baseline-tool",
                    "smll",
                    "--cases",
                    str(cases),
                    "--contract",
                    str(root),
                    "--jobs",
                    "1",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )

            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("baseline comparison: 1", proc.stdout)

    def test_identical_baseline_and_candidate_pass(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            binary = root / "tapas"
            binary.write_text("#!/bin/sh\nprintf 'same output\\n'\n", encoding="utf-8")
            binary.chmod(0o755)
            cases = root / "cases.json"
            cases.write_text(
                """{
  "cases": [{
    "id": "comparison:identical",
    "oracle": "smll",
    "mode": "pipe",
    "argv": ["unused"],
    "stdin": {"base64": ""},
    "env": {"set": {}, "unset": []},
    "expect": {
      "termination": {"exit_code": 0, "signal": null},
      "stdout": {"facts": ["same output"], "byte_exact": false},
      "stderr": {"facts": [], "byte_exact": false},
      "incomplete_output": {"diagnostic_facts": []}
    }
  }]
}
""",
                encoding="utf-8",
            )

            proc = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "parity.py"),
                    "--binary",
                    str(binary),
                    "--baseline-binary",
                    str(binary),
                    "--tool",
                    "tapas",
                    "--cases",
                    str(cases),
                    "--contract",
                    str(root),
                    "--jobs",
                    "1",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )

            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("baseline comparison: 1", proc.stdout)
            self.assertIn("all characterization cases passed", proc.stdout)

    def test_stdout_stderr_and_exit_mismatches_are_actionable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = root / "baseline"
            baseline.write_text(
                "#!/bin/sh\nprintf 'baseline out'\nprintf 'baseline err' >&2\nexit 7\n",
                encoding="utf-8",
            )
            baseline.chmod(0o755)
            candidate = root / "candidate"
            candidate.write_text(
                "#!/bin/sh\nprintf 'candidate out'\nprintf 'candidate err' >&2\nexit 9\n",
                encoding="utf-8",
            )
            candidate.chmod(0o755)
            cases = root / "cases.json"
            cases.write_text(
                """{
  "cases": [{
    "id": "comparison:different",
    "oracle": "smll",
    "mode": "pipe",
    "argv": ["unused"],
    "stdin": {"base64": ""},
    "env": {"set": {}, "unset": []},
    "expect": {
      "termination": {"exit_code": null, "signal": null},
      "stdout": {"facts": [], "byte_exact": false},
      "stderr": {"facts": [], "byte_exact": false},
      "incomplete_output": {"diagnostic_facts": []}
    }
  }]
}
""",
                encoding="utf-8",
            )

            proc = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "parity.py"),
                    "--binary",
                    str(candidate),
                    "--baseline-binary",
                    str(baseline),
                    "--tool",
                    "tapas",
                    "--cases",
                    str(cases),
                    "--contract",
                    str(root),
                    "--jobs",
                    "1",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )

            self.assertEqual(proc.returncode, 1)
            self.assertIn("FAIL comparison:different", proc.stderr)
            self.assertIn("exit differs (baseline 7, candidate 9)", proc.stderr)
            self.assertRegex(
                proc.stderr,
                r"stdout differs \(baseline 12 bytes, candidate 13 bytes; "
                r"first difference at byte 0: baseline=0x62, candidate=0x63\)",
            )
            self.assertRegex(
                proc.stderr,
                r"stderr differs \(baseline 12 bytes, candidate 13 bytes; "
                r"first difference at byte 0: baseline=0x62, candidate=0x63\)",
            )


if __name__ == "__main__":
    unittest.main()
