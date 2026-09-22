import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

const KILL_GRACE_MS = 2_000;
const HARD_DEADLINE_MS = 5_000;

/// Builds the harness command runner for one test environment.
///
/// Each command runs in its own process group on Unix so a timeout can
/// terminate the whole tree, and the promise settles with an explicit
/// `timedOut` flag so a child that handles SIGTERM and exits zero is still
/// reported as a failure.
export function createRunner({ repository, environment, artifactRoot, timeoutMs }) {
  return async function run(command, args, options = {}) {
    const label = options.label ?? command.split("/").at(-1);
    const detached = process.platform !== "win32";
    const result = await new Promise((resolvePromise, rejectPromise) => {
      const child = spawn(command, args, {
        cwd: options.cwd ?? repository,
        env: environment,
        stdio: ["ignore", "pipe", "pipe"],
        detached,
      });
      const stdout = [];
      const stderr = [];
      let settled = false;
      let timedOut = false;
      let timeout;
      let escalate;
      let hardDeadline;
      const clearTimers = () => {
        clearTimeout(timeout);
        clearTimeout(escalate);
        clearTimeout(hardDeadline);
      };
      const killTree = (signal) => {
        if (detached && child.pid) {
          try {
            process.kill(-child.pid, signal);
          } catch (error) {
            if (error.code !== "ESRCH") throw error;
          }
        } else {
          child.kill(signal);
        }
      };
      const finish = (code, signal) => {
        if (settled) return;
        settled = true;
        clearTimers();
        resolvePromise({
          code,
          signal,
          timedOut,
          stdout: Buffer.concat(stdout).toString("utf8"),
          stderr: Buffer.concat(stderr).toString("utf8"),
        });
      };
      timeout = setTimeout(() => {
        timedOut = true;
        killTree("SIGTERM");
        escalate = setTimeout(() => killTree("SIGKILL"), KILL_GRACE_MS);
        hardDeadline = setTimeout(() => finish(null, "SIGKILL"), HARD_DEADLINE_MS);
      }, timeoutMs);
      child.stdout.on("data", (chunk) => stdout.push(chunk));
      child.stderr.on("data", (chunk) => stderr.push(chunk));
      child.on("error", (error) => {
        if (settled) return;
        settled = true;
        clearTimers();
        rejectPromise(error);
      });
      child.on("close", (code, signal) => finish(code, signal));
    });

    await Promise.all([
      writeFile(join(artifactRoot, `${label}.stdout.log`), result.stdout),
      writeFile(join(artifactRoot, `${label}.stderr.log`), result.stderr),
    ]);
    assert.ok(
      !result.timedOut && result.code === 0,
      `${label} ${result.timedOut ? "timed out" : `exited with ${result.code ?? result.signal}`}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`,
    );
    return result;
  };
}
