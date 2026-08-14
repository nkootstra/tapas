from __future__ import annotations

import json
import pathlib
import subprocess
import tempfile
import textwrap
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/publish-pr.yml"
START = "# pr-build-metadata-validator:start"
END = "# pr-build-metadata-validator:end"


class PrBuildMetadataTests(unittest.TestCase):
    def validator(self) -> str:
        workflow = WORKFLOW.read_text(encoding="utf-8")
        return textwrap.dedent(workflow.split(START, 1)[1].split(END, 1)[0])

    def run_validator(
        self, metadata: list[dict[str, object]], source_sha: str
    ) -> tuple[subprocess.CompletedProcess[str], str]:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            github_env = root / "github-env"
            paths = []
            for index, value in enumerate(metadata):
                path = root / f"metadata-{index}.json"
                path.write_text(json.dumps(value), encoding="utf-8")
                paths.append(str(path))
            result = subprocess.run(
                ["python3", "-", source_sha, str(github_env), *paths],
                input=self.validator(),
                text=True,
                capture_output=True,
                check=False,
            )
            environment = github_env.read_text() if github_env.exists() else ""
            return result, environment

    def test_accepts_matching_target_metadata(self) -> None:
        source_sha = "a65474d7" + "a" * 32
        metadata = {
            "source_sha": source_sha,
            "version": "0.5.0",
            "version_label": "0.5.0-dev.a65474d7",
        }

        result, environment = self.run_validator([metadata, dict(metadata)], source_sha)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(environment, "TAPAS_BUILD_LABEL=0.5.0-dev.a65474d7\n")

    def test_rejects_invalid_or_inconsistent_metadata(self) -> None:
        source_sha = "a65474d7" + "a" * 32
        valid = {
            "source_sha": source_sha,
            "version": "0.5.0",
            "version_label": "0.5.0-dev.a65474d7",
        }
        invalid_cases = (
            [],
            [{**valid, "source_sha": "b" * 40}],
            [{**valid, "version": "0.5.0-rc.1"}],
            [{**valid, "version_label": "0.5.0-dev.00000000"}],
            [valid, {**valid, "version": "0.6.0", "version_label": "0.6.0-dev.a65474d7"}],
        )

        for metadata in invalid_cases:
            with self.subTest(metadata=metadata):
                result, _ = self.run_validator(metadata, source_sha)
                self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
