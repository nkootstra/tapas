import assert from "node:assert/strict";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { createRunner } from "./run.mjs";

const environment = { PATH: process.env.PATH ?? "/usr/bin:/bin" };

async function runnerFor(timeoutMs) {
  const artifactRoot = await mkdtemp(join(tmpdir(), "tapas-runner-test-"));
  return createRunner({
    repository: process.cwd(),
    environment,
    artifactRoot,
    timeoutMs,
  });
}

test("a timeout terminates the whole process group", async () => {
  const directory = await mkdtemp(join(tmpdir(), "tapas-runner-tree-"));
  const pidFile = join(directory, "descendant.pid");
  const run = await runnerFor(300);

  await assert.rejects(
    run("/bin/sh", ["-c", `sleep 5 & echo $! > '${pidFile}'; sleep 5`]),
    /timed out/,
  );

  const descendant = Number((await readFile(pidFile, "utf8")).trim());
  let gone = false;
  for (let attempt = 0; attempt < 150 && !gone; attempt += 1) {
    try {
      process.kill(descendant, 0);
      await new Promise((resolve) => setTimeout(resolve, 20));
    } catch (error) {
      gone = error.code === "ESRCH";
    }
  }
  if (!gone) {
    try {
      process.kill(descendant, "SIGKILL");
    } catch {
      // already gone
    }
  }
  assert.ok(gone, "the descendant survived the timeout");
});

test("a command that ignores SIGTERM is still reported as a timeout", async () => {
  const run = await runnerFor(300);
  await assert.rejects(
    run("/bin/sh", ["-c", "trap '' TERM; while true; do sleep 1; done"]),
    /timed out/,
  );
});

test("a clean command resolves", async () => {
  const run = await runnerFor(5_000);
  const result = await run("/bin/sh", ["-c", "printf hello"]);
  assert.equal(result.code, 0);
  assert.equal(result.stdout, "hello");
});

test("a spawn error rejects", async () => {
  const run = await runnerFor(5_000);
  await assert.rejects(run("/nonexistent/tapas-missing-command", []));
});
