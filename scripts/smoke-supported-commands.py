#!/usr/bin/env python3
"""Exercise Tapas against live CLI output in disposable projects."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable, Iterable, Sequence
from enum import Enum
from typing import NamedTuple


class Completed(NamedTuple):
    returncode: int
    stdout: bytes
    stderr: bytes


class Status(str, Enum):
    PASSED = "passed"
    FAILED = "failed"
    SKIPPED = "skipped"


class Result(NamedTuple):
    name: str
    status: Status
    raw_bytes: int = 0
    compact_bytes: int = 0
    raw_lines: int = 0
    compact_lines: int = 0
    facts: tuple[str, ...] = ()
    detail: str = ""


class Unavailable(RuntimeError):
    pass


class VerificationError(RuntimeError):
    pass


MINIMUM_REDUCTION = 0.10


def verify(condition: bool, detail: str) -> None:
    if not condition:
        raise VerificationError(detail)


def evaluate(
    name: str,
    raw: Completed,
    compact: Completed,
    *,
    facts: Sequence[bytes] = (),
    summary_facts: Sequence[bytes] = (),
    minimum_reduction: float = MINIMUM_REDUCTION,
    exact: bool = False,
    expect_failure: bool = False,
) -> Result:
    verify(
        compact.returncode == raw.returncode,
        f"{name}: exit changed from {raw.returncode} to {compact.returncode}",
    )
    if expect_failure:
        verify(raw.returncode != 0, f"{name}: command unexpectedly succeeded")
    else:
        verify(raw.returncode == 0, f"{name}: command unexpectedly failed")
    if exact:
        verify(compact.stdout == raw.stdout, f"{name}: stdout changed in exact mode")
        verify(compact.stderr == raw.stderr, f"{name}: stderr changed in exact mode")
    else:
        raw_size = len(raw.stdout) + len(raw.stderr)
        compact_size = len(compact.stdout) + len(compact.stderr)
        reduction = 1 - compact_size / raw_size if raw_size else 0
        verify(
            reduction >= minimum_reduction,
            f"{name}: Tapas did not reduce output by at least "
            f"{minimum_reduction:.0%} ({raw_size} -> {compact_size} bytes)",
        )
        raw_output = raw.stdout + raw.stderr
        compact_output = compact.stdout + compact.stderr
        for fact in facts:
            verify(fact in raw_output, f"{name}: raw output missing fact {fact!r}")
            verify(
                fact in compact_output,
                f"{name}: compact output missing fact {fact!r}",
            )
        for fact in summary_facts:
            verify(
                fact in compact_output,
                f"{name}: compact output missing summary {fact!r}",
            )
    return Result(
        name=name,
        status=Status.PASSED,
        raw_bytes=len(raw.stdout) + len(raw.stderr),
        compact_bytes=len(compact.stdout) + len(compact.stderr),
        raw_lines=raw.stdout.count(b"\n") + raw.stderr.count(b"\n"),
        compact_lines=compact.stdout.count(b"\n") + compact.stderr.count(b"\n"),
        facts=tuple(
            fact.decode("utf-8", "replace") for fact in (*facts, *summary_facts)
        ),
        detail="byte-exact" if exact else "compact and facts retained",
    )


def validate_summary(results: Sequence[Result], *, require_all: bool) -> None:
    failures = [result.detail for result in results if result.status == Status.FAILED]
    if require_all:
        failures.extend(
            result.detail for result in results if result.status == Status.SKIPPED
        )
    verify(not failures, "; ".join(failures))


class Smoke:
    def __init__(self, root: pathlib.Path, tapas: pathlib.Path) -> None:
        self.root = root
        self.tapas = tapas
        self.environment = os.environ.copy()
        self.environment.pop("FORCE_COLOR", None)
        self.environment.pop("NO_COLOR", None)
        self.environment["CI"] = "1"
        self.docker_available: bool | None = None

    def need(self, *commands: str) -> None:
        missing = [command for command in commands if shutil.which(command) is None]
        if missing:
            raise Unavailable(f"missing commands: {', '.join(missing)}")

    def require_docker(self, detail: str = "Docker daemon unavailable") -> None:
        if self.docker_available is None:
            healthy = self.run(
                ["docker", "info", "--format", "{{.ServerVersion}}"], self.root
            )
            self.docker_available = healthy.returncode == 0
        if not self.docker_available:
            raise Unavailable(detail)

    def run(
        self,
        argv: Sequence[str | os.PathLike[str]],
        cwd: pathlib.Path,
        *,
        env: dict[str, str] | None = None,
        timeout: int = 180,
    ) -> Completed:
        merged = self.environment.copy()
        if env:
            merged.update(env)
        process = subprocess.run(
            [os.fspath(argument) for argument in argv],
            cwd=cwd,
            env=merged,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
        return Completed(process.returncode, process.stdout, process.stderr)

    def cleanup(
        self,
        actions: Sequence[
            tuple[str, Sequence[str | os.PathLike[str]], pathlib.Path, int]
        ],
    ) -> None:
        failures: list[str] = []
        for label, argv, cwd, timeout in actions:
            try:
                result = self.run(argv, cwd, timeout=timeout)
            except (subprocess.SubprocessError, OSError) as error:
                failures.append(f"{label}: {error}")
                continue
            if result.returncode != 0:
                detail = (result.stderr or result.stdout).decode("utf-8", "replace").strip()
                failures.append(f"{label}: {detail or f'exit {result.returncode}'}")
        if not failures:
            return
        detail = "cleanup failed: " + "; ".join(failures)
        active = sys.exc_info()[1]
        if active is not None and hasattr(active, "add_note"):
            active.add_note(detail)
        elif active is not None:
            print(detail, file=sys.stderr)
        else:
            raise VerificationError(detail)

    def pair(
        self,
        name: str,
        argv: Sequence[str | os.PathLike[str]],
        cwd: pathlib.Path,
        *,
        facts: Sequence[bytes] = (),
        summary_facts: Sequence[bytes] = (),
        minimum_reduction: float = MINIMUM_REDUCTION,
        exact: bool = False,
        expect_failure: bool = False,
        env: dict[str, str] | None = None,
        timeout: int = 180,
    ) -> Result:
        raw = self.run(argv, cwd, env=env, timeout=timeout)
        compact = self.run(
            [self.tapas, *argv], cwd, env=env, timeout=timeout
        )
        return evaluate(
            name,
            raw,
            compact,
            facts=facts,
            summary_facts=summary_facts,
            minimum_reduction=minimum_reduction,
            exact=exact,
            expect_failure=expect_failure,
        )

    def pip(self) -> list[Result]:
        self.need("python3", "pip3")
        work = self.root / "pip"
        work.mkdir()
        created = self.run(["python3", "-m", "venv", "venv"], work)
        if created.returncode != 0:
            raise VerificationError(f"pip: venv creation failed: {created.stderr!r}")
        pip = work / "venv" / "bin" / "pip"
        return [
            self.pair("pip-list", [pip, "list"], work, facts=(b"pip",)),
            self.pair(
                "pip-json-exact", [pip, "list", "--format=json"], work, exact=True
            ),
            self.pair(
                "pip-failure-exact",
                [pip, "install", "--no-index", "tapas-live-missing-package"],
                work,
                exact=True,
                expect_failure=True,
            ),
            self.pair("pip3-list", ["pip3", "list"], work, facts=(b"pip",)),
        ]

    def uv(self) -> list[Result]:
        self.need("uv")
        raw_dir = self.root / "uv-raw"
        tapas_dir = self.root / "uv-tapas"
        for directory in (raw_dir, tapas_dir):
            directory.mkdir()
            (directory / "pyproject.toml").write_text(
                "[project]\n"
                'name = "tapas-live-smoke"\n'
                'version = "0.1.0"\n'
                'requires-python = ">=3.12"\n'
                'dependencies = ["rich==14.2.0", "click==8.3.0"]\n',
                encoding="utf-8",
            )
        raw_env = {"UV_CACHE_DIR": os.fspath(self.root / "uv-raw-cache")}
        tapas_env = {"UV_CACHE_DIR": os.fspath(self.root / "uv-tapas-cache")}
        raw = self.run(["uv", "sync"], raw_dir, env=raw_env)
        compact = self.run([self.tapas, "uv", "sync"], tapas_dir, env=tapas_env)
        result = evaluate("uv-sync", raw, compact, facts=(b"Resolved", b"Installed"))
        exact = self.pair(
            "uv-pip-list-json-exact",
            ["uv", "pip", "list", "--python", ".venv/bin/python", "--format", "json"],
            raw_dir,
            exact=True,
            env=raw_env,
        )
        return [result, exact]

    def node(self) -> list[Result]:
        self.need("npm", "npx")
        work = self.root / "node"
        work.mkdir()
        (work / "package.json").write_text(
            '{"private":true,"type":"commonjs"}\n', encoding="utf-8"
        )
        installed = self.run(
            [
                "npm",
                "install",
                "--no-save",
                "--ignore-scripts",
                "vite",
                "esbuild",
                "@playwright/test",
            ],
            work,
        )
        if installed.returncode != 0:
            raise VerificationError(f"node: package install failed: {installed.stderr!r}")
        (work / "index.html").write_text(
            '<div id="app"></div><script type="module" src="/main.js"></script>\n',
            encoding="utf-8",
        )
        (work / "main.js").write_text(
            "const rows = Array.from({length: 200}, (_, i) => `row-${i}`);\n"
            "document.querySelector('#app').textContent = rows.join(', ');\n",
            encoding="utf-8",
        )
        (work / "app.js").write_text(
            "console.log(Array.from({length: 100}, (_, i) => i).join(','));\n",
            encoding="utf-8",
        )
        (work / "smoke.spec.js").write_text(
            "const { test, expect } = require('@playwright/test');\n"
            "for (let i = 0; i < 12; i += 1) {\n"
            "  test(`case ${i}`, async () => expect(i).toBe(i));\n"
            "}\n",
            encoding="utf-8",
        )
        return [
            self.pair(
                "vite-build",
                ["npx", "vite", "build"],
                work,
                facts=(b"vite v", b"built in", b"dist/"),
            ),
            self.pair(
                "esbuild",
                ["npx", "esbuild", "app.js", "--bundle", "--outfile=out.js"],
                work,
                facts=(b"out.js", b"Done in"),
            ),
            self.pair(
                "playwright",
                [
                    "npx",
                    "playwright",
                    "test",
                    "smoke.spec.js",
                    "--reporter=list",
                ],
                work,
                facts=(b"Running 12 tests", b"12 passed"),
            ),
        ]

    def cmake(self) -> list[Result]:
        self.need("uvx")
        work = self.root / "cmake"
        work.mkdir()
        (work / "CMakeLists.txt").write_text(
            "cmake_minimum_required(VERSION 3.16)\n"
            "project(tapas_live_smoke C)\n"
            "enable_testing()\n"
            "add_executable(smoke main.c)\n"
            "foreach(index RANGE 1 12)\n"
            "  add_test(NAME smoke_${index} COMMAND smoke)\n"
            "endforeach()\n",
            encoding="utf-8",
        )
        (work / "main.c").write_text(
            '#include <stdio.h>\nint main(void) { puts("ok"); return 0; }\n',
            encoding="utf-8",
        )
        warmed = self.run(["uvx", "--from", "cmake", "cmake", "--version"], work)
        if warmed.returncode != 0:
            raise VerificationError(f"cmake: uvx warmup failed: {warmed.stderr!r}")
        raw = self.run(
            ["uvx", "--from", "cmake", "cmake", "-S", ".", "-B", "build-raw"],
            work,
        )
        compact = self.run(
            [
                self.tapas,
                "uvx",
                "--from",
                "cmake",
                "cmake",
                "-S",
                ".",
                "-B",
                "build-tapas",
            ],
            work,
        )
        configured = evaluate(
            "cmake-configure",
            raw,
            compact,
            facts=(b"Configuring done", b"Generating done", b"Build files"),
        )
        built = self.run(
            ["uvx", "--from", "cmake", "cmake", "--build", "build-raw"], work
        )
        if built.returncode != 0:
            raise VerificationError(f"cmake: build failed: {built.stderr!r}")
        tested = self.pair(
            "ctest",
            ["uvx", "--from", "cmake", "ctest", "--test-dir", "build-raw"],
            work,
            facts=(b"100% tests passed", b"Total Test time"),
        )
        return [configured, tested]

    def grep(self) -> list[Result]:
        self.need("grep")
        work = self.root / "grep"
        work.mkdir()
        for name in ("alpha.txt", "beta.txt"):
            stem = name.split(".", 1)[0]
            (work / name).write_text(
                "".join(f"needle {stem} {index:02d}\n" for index in range(1, 13)),
                encoding="utf-8",
            )
        return [
            self.pair(
                "grep-multifile",
                ["grep", "-H", "needle", "alpha.txt", "beta.txt"],
                work,
                facts=(b"alpha.txt:needle alpha 01", b"beta.txt"),
                summary_facts=(b"9 more matches",),
            ),
            self.pair(
                "grep-count-exact",
                ["grep", "-c", "needle", "alpha.txt", "beta.txt"],
                work,
                exact=True,
            ),
        ]

    def bat(self) -> list[Result]:
        self.need("bat")
        work = self.root / "bat"
        work.mkdir()
        source = "fn main() {\n    println!(\"start\");\n}\n\n" + "".join(
            f'fn helper_{index:02d}() {{\n'
            f'    println!("helper {index}");\n'
            '    println!("still working");\n'
            '    println!("done");\n'
            '}\n'
            for index in range(1, 41)
        )
        (work / "sample.rs").write_text(source, encoding="utf-8")
        results = [
            self.pair(
                "bat-code",
                ["bat", "sample.rs"],
                work,
                facts=(b"fn main()", b"helper_40"),
                summary_facts=(b"lines",),
            ),
            self.pair(
                "bat-option-exact",
                ["bat", "--style=plain", "sample.rs"],
                work,
                exact=True,
            ),
        ]
        bin_dir = work / "bin"
        bin_dir.mkdir()
        (bin_dir / "batcat").symlink_to(pathlib.Path(shutil.which("bat") or "bat"))
        env = {"PATH": f"{bin_dir}{os.pathsep}{self.environment['PATH']}"}
        results.append(
            self.pair(
                "batcat-code",
                ["batcat", "sample.rs"],
                work,
                facts=(b"fn main()", b"helper_40"),
                summary_facts=(b"lines",),
                env=env,
            )
        )
        return results

    def helm(self) -> list[Result]:
        self.need("helm", "kind", "docker")
        self.require_docker("Docker daemon unavailable for Helm Kind cluster")
        work = self.root / "helm"
        work.mkdir()
        cluster = f"tapas-live-{os.getpid()}-{int(time.time())}"
        kubeconfig = work / "kubeconfig"
        env = {"KUBECONFIG": os.fspath(kubeconfig)}
        try:
            created = self.run(
                [
                    "kind",
                    "create",
                    "cluster",
                    "--name",
                    cluster,
                    "--kubeconfig",
                    kubeconfig,
                    "--wait",
                    "90s",
                ],
                work,
                timeout=240,
            )
            if created.returncode != 0:
                raise VerificationError(f"helm: Kind creation failed: {created.stderr!r}")
            chart = self.run(["helm", "create", "chart"], work, env=env)
            installed = self.run(["helm", "install", "demo", "chart"], work, env=env)
            if chart.returncode != 0 or installed.returncode != 0:
                raise VerificationError(
                    f"helm: chart setup failed: {chart.stderr!r} {installed.stderr!r}"
                )
            return [
                self.pair(
                    "helm-list",
                    ["helm", "list"],
                    work,
                    facts=(b"demo", b"deployed", b"chart-"),
                    env=env,
                ),
                self.pair(
                    "helm-history",
                    ["helm", "history", "demo"],
                    work,
                    facts=(b"deployed", b"Install complete"),
                    env=env,
                ),
                self.pair(
                    "helm-status",
                    ["helm", "status", "demo"],
                    work,
                    facts=(b"NAME: demo", b"STATUS: deployed", b"NOTES:"),
                    minimum_reduction=0.05,
                    env=env,
                ),
                self.pair(
                    "helm-json-exact",
                    ["helm", "list", "--output", "json"],
                    work,
                    exact=True,
                    env=env,
                ),
            ]
        finally:
            self.cleanup(
                [
                    (
                        "Kind cluster",
                        ["kind", "delete", "cluster", "--name", cluster],
                        work,
                        120,
                    )
                ]
            )

    def docker(self) -> list[Result]:
        self.need("docker", "docker-compose")
        self.require_docker()
        work = self.root / "docker"
        work.mkdir()
        (work / "Dockerfile").write_text(
            "# syntax=docker/dockerfile:1\n"
            "FROM alpine:3.22\n"
            "RUN printf 'alpha\\nbeta\\ngamma\\n' > /message.txt\n"
            'CMD ["cat", "/message.txt"]\n',
            encoding="utf-8",
        )
        (work / "compose.yml").write_text(
            "services:\n"
            "  smoke:\n"
            "    image: alpine:3.22\n"
            '    command: ["sleep", "60"]\n',
            encoding="utf-8",
        )
        prefix = f"tapas-live-{os.getpid()}-{int(time.time())}"
        raw_image = f"{prefix}-raw"
        compact_image = f"{prefix}-compact"
        container = f"{prefix}-stats"
        project = prefix.replace("-", "")
        compose = [
            "docker-compose",
            "--project-name",
            project,
            "--file",
            "compose.yml",
        ]
        raw_image_created = False
        compact_image_created = False
        container_started = False
        try:
            pulled = self.run(["docker", "pull", "alpine:3.22"], work)
            if pulled.returncode != 0:
                raise VerificationError(f"docker: image pull failed: {pulled.stderr!r}")
            raw_build = self.run(
                [
                    "docker",
                    "build",
                    "--no-cache",
                    "--progress=plain",
                    "--tag",
                    raw_image,
                    ".",
                ],
                work,
            )
            raw_image_created = raw_build.returncode == 0
            compact_build = self.run(
                [
                    self.tapas,
                    "docker",
                    "build",
                    "--no-cache",
                    "--progress=plain",
                    "--tag",
                    compact_image,
                    ".",
                ],
                work,
            )
            compact_image_created = compact_build.returncode == 0
            built = evaluate(
                "docker-build",
                raw_build,
                compact_build,
                facts=(b"docker.io/library/tapas-live-",),
            )
            started = self.run(
                ["docker", "run", "--detach", "--name", container, "alpine:3.22", "sleep", "60"],
                work,
            )
            if started.returncode != 0:
                raise VerificationError(
                    f"docker: container start failed: {started.stderr!r}"
                )
            container_started = True
            stats = self.pair(
                "docker-stats",
                ["docker", "stats", "--no-stream", container],
                work,
                facts=(container.encode(),),
            )
            composed = self.run([*compose, "up", "--detach"], work)
            if composed.returncode != 0:
                raise VerificationError(
                    f"docker-compose: up failed: {composed.stderr!r}"
                )
            compose_stats = self.pair(
                "docker-compose-stats",
                [*compose, "stats", "--no-stream"],
                work,
                facts=(project.encode(),),
            )
            return [built, stats, compose_stats]
        finally:
            actions: list[
                tuple[str, Sequence[str | os.PathLike[str]], pathlib.Path, int]
            ] = []
            if container_started:
                actions.append(
                    (
                        "Docker stats container",
                        ["docker", "container", "rm", "--force", container],
                        work,
                        180,
                    )
                )
            actions.append(
                ("Docker Compose project", [*compose, "down", "--remove-orphans"], work, 180)
            )
            images = [
                image
                for image, created in (
                    (raw_image, raw_image_created),
                    (compact_image, compact_image_created),
                )
                if created
            ]
            if images:
                actions.append(
                    (
                        "Docker build images",
                        ["docker", "image", "rm", "--force", *images],
                        work,
                        180,
                    )
                )
            self.cleanup(actions)


Case = Callable[[Smoke], list[Result]]
CASES: dict[str, Case] = {
    "pip": Smoke.pip,
    "uv": Smoke.uv,
    "node": Smoke.node,
    "cmake": Smoke.cmake,
    "grep": Smoke.grep,
    "bat": Smoke.bat,
    "helm": Smoke.helm,
    "docker": Smoke.docker,
}


def result_dict(result: Result) -> dict[str, object]:
    return result._asdict()


def print_text(results: Iterable[Result]) -> None:
    for result in results:
        if result.status == Status.PASSED:
            metric = (
                result.detail
                if result.detail == "byte-exact"
                else f"{result.raw_bytes}->{result.compact_bytes} bytes, "
                f"{result.raw_lines}->{result.compact_lines} lines"
            )
            facts = f"; facts: {', '.join(result.facts)}" if result.facts else ""
            print(f"PASS {result.name}: {metric}{facts}")
        else:
            print(f"{result.status.upper()} {result.name}: {result.detail}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, default=pathlib.Path("target/debug/tapas"))
    parser.add_argument("--case", action="append", choices=sorted(CASES))
    parser.add_argument("--list", action="store_true", help="list case groups and exit")
    parser.add_argument("--require-all", action="store_true", help="fail when a tool is unavailable")
    parser.add_argument("--format", choices=("text", "json"), default="text")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.list:
        print("\n".join(CASES))
        return 0
    tapas = args.binary.resolve()
    if not tapas.is_file():
        print(f"Tapas binary not found: {tapas}", file=sys.stderr)
        return 2
    selected = args.case or list(CASES)
    results: list[Result] = []
    with tempfile.TemporaryDirectory(prefix="tapas-live-smoke-") as temporary:
        smoke = Smoke(pathlib.Path(temporary), tapas)
        for name in selected:
            try:
                results.extend(CASES[name](smoke))
            except Unavailable as error:
                results.append(Result(name, Status.SKIPPED, detail=str(error)))
            except (VerificationError, subprocess.SubprocessError, OSError) as error:
                results.append(Result(name, Status.FAILED, detail=str(error)))
    if args.format == "json":
        print(json.dumps([result_dict(result) for result in results], indent=2))
    else:
        print_text(results)
    try:
        validate_summary(results, require_all=args.require_all)
    except VerificationError as error:
        print(f"live smoke failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
