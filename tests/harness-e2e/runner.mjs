import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { LLMock } from "@copilotkit/aimock";

const harness = process.env.TAPAS_HARNESS;
assert.match(harness ?? "", /^(claude|codex|opencode)$/, "set TAPAS_HARNESS");

const testRoot = dirname(fileURLToPath(import.meta.url));
const repository = resolve(testRoot, "../..");
const tapas = resolve(process.env.TAPAS_BINARY ?? join(repository, "target/debug/tapas"));
const binaries = join(testRoot, "node_modules/.bin");
const prompt = "TAPAS_HARNESS_E2E: inspect this repository by running git status exactly once.";
const sentinel = `TAPAS_E2E_OK_${harness.toUpperCase()}`;
const toolCallId = "call_tapas_git_status";
const timeoutMs = Number(process.env.TAPAS_HARNESS_TIMEOUT_MS ?? 90_000);

const temporaryRoot = await mkdtemp(join(tmpdir(), `tapas-${harness}-e2e-`));
const home = join(temporaryRoot, "home");
const workspace = join(temporaryRoot, "workspace");
const codexHome = join(home, ".codex");
const xdgConfigHome = join(home, ".config");
const xdgDataHome = join(home, ".local/share");
const xdgCacheHome = join(home, ".cache");
const artifactRoot = resolve(
  process.env.TAPAS_HARNESS_ARTIFACTS ?? join(temporaryRoot, "artifacts"),
  harness,
);

await Promise.all([
  mkdir(home, { recursive: true }),
  mkdir(workspace, { recursive: true }),
  mkdir(codexHome, { recursive: true }),
  mkdir(xdgConfigHome, { recursive: true }),
  mkdir(xdgDataHome, { recursive: true }),
  mkdir(xdgCacheHome, { recursive: true }),
  mkdir(artifactRoot, { recursive: true }),
]);

const environment = {
  PATH: `${binaries}:${process.env.PATH ?? "/usr/local/bin:/usr/bin:/bin"}`,
  HOME: home,
  CODEX_HOME: codexHome,
  XDG_CONFIG_HOME: xdgConfigHome,
  XDG_DATA_HOME: xdgDataHome,
  XDG_CACHE_HOME: xdgCacheHome,
  LANG: "C.UTF-8",
  LC_ALL: "C.UTF-8",
  CI: "true",
  NO_COLOR: "1",
  TAPAS_AIMOCK_API_KEY: "tapas-mock-key",
  ANTHROPIC_API_KEY: "tapas-mock-key",
  CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
  DISABLE_AUTOUPDATER: "1",
  DISABLE_TELEMETRY: "1",
  DISABLE_ERROR_REPORTING: "1",
  OPENCODE_DISABLE_AUTOUPDATE: "1",
};

let succeeded = false;
const mock = new LLMock({ port: 0, strict: true, journalMaxEntries: 20 });
let observedRequests = [];
const adapter = createHarnessAdapter(harness);

try {
  await run(adapter.binary, ["--version"], { label: `${harness}-version` });
  await initializeRepository();
  await mock.start();
  environment.ANTHROPIC_BASE_URL = mock.url;
  configureFixtures();
  await adapter.configure();

  await runTapas("--setup", harness);
  await saveHarnessConfiguration("configured");
  const configured = await runHarness("configured");
  assert.match(configured.stdout, new RegExp(sentinel));
  await saveJournal("configured");
  assertToolResult(mock.getRequests(), {
    includes: "# tapas-ci",
    excludes: ["On branch tapas-ci", "nothing to commit"],
    phase: "configured",
  });

  await runTapas("--unsetup", harness);
  await saveHarnessConfiguration("unconfigured");
  mock.clearRequests();
  mock.resetMatchCounts();
  observedRequests = [];

  const unconfigured = await runHarness("unconfigured");
  assert.match(unconfigured.stdout, new RegExp(sentinel));
  await saveJournal("unconfigured");
  assertToolResult(mock.getRequests(), {
    includes: "On branch tapas-ci",
    excludes: ["# tapas-ci"],
    phase: "unconfigured",
  });

  succeeded = true;
  process.stdout.write(`${harness}: real harness setup and unsetup contracts passed\n`);
} catch (error) {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.stderr.write(`artifacts: ${artifactRoot}\n`);
  process.stderr.write(`isolated test root retained at: ${temporaryRoot}\n`);
  process.exitCode = 1;
} finally {
  await mock.stop().catch(() => {});
  if (succeeded && !process.env.TAPAS_KEEP_HARNESS_TEMP) {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

async function initializeRepository() {
  await run("git", ["init", "--initial-branch=tapas-ci"], { cwd: workspace });
  await run("git", ["config", "user.name", "Tapas CI"], { cwd: workspace });
  await run("git", ["config", "user.email", "tapas-ci@example.invalid"], { cwd: workspace });
  await run("git", ["commit", "--allow-empty", "--message", "initial"], { cwd: workspace });
}

function configureFixtures() {
  mock.onToolResult(toolCallId, (request) => {
    observedRequests.push(structuredClone(request));
    return { content: sentinel };
  });
  mock.on(
    { userMessage: prompt, hasToolResult: false },
    (request) => {
      observedRequests.push(structuredClone(request));
      return { toolCalls: [shellToolCall(request)] };
    },
  );
}

function shellToolCall(request) {
  const tools = request.tools ?? [];
  const tool = tools.find((candidate) =>
    adapter.shellToolNames.includes(candidate.function?.name),
  );
  assert.ok(
    tool,
    `${harness}: shell tool not advertised; received ${tools
      .map((candidate) => candidate.function?.name)
      .filter(Boolean)
      .join(", ")}`,
  );

  const name = tool.function.name;
  const properties = tool.function.parameters?.properties ?? {};
  const argumentsObject = {};
  if (properties.cmd) {
    argumentsObject.cmd = "git status";
  } else if (properties.command?.type === "array") {
    argumentsObject.command = ["git", "status"];
  } else {
    argumentsObject.command = "git status";
  }
  if (tool.function.parameters?.required?.includes("description")) {
    argumentsObject.description = "Read repository status";
  }

  return {
    id: toolCallId,
    name,
    arguments: JSON.stringify(argumentsObject),
  };
}

function createHarnessAdapter(name) {
  switch (name) {
    case "claude":
      return createClaudeAdapter();
    case "codex":
      return createCodexAdapter();
    case "opencode":
      return createOpenCodeAdapter();
    default:
      throw new Error(`unsupported harness: ${name}`);
  }
}

function createClaudeAdapter() {
  return {
    binary: join(binaries, "claude"),
    shellToolNames: ["Bash"],
    configure: async () => {},
    invocation: () => [
      "--print",
      prompt,
      "--model",
      "claude-sonnet-4-6",
      "--max-turns",
      "2",
      "--output-format",
      "stream-json",
      "--verbose",
      "--include-hook-events",
      "--tools",
      "Bash",
      "--disable-slash-commands",
      "--no-session-persistence",
      "--dangerously-skip-permissions",
      "--setting-sources",
      "user",
    ],
    artifactPaths: () => [[join(home, ".claude/settings.json"), "settings.json"]],
  };
}

function createCodexAdapter() {
  return {
    binary: join(binaries, "codex"),
    shellToolNames: ["exec_command", "shell"],
    configure: async () => {
      const config = `model = "tapas-ci"
model_provider = "aimock"
model_reasoning_effort = "low"
model_reasoning_summary = "none"
model_supports_reasoning_summaries = false
web_search = "disabled"

[model_providers.aimock]
name = "aimock"
base_url = "${mock.url}/v1"
env_key = "TAPAS_AIMOCK_API_KEY"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
`;
      await writeFile(join(codexHome, "config.toml"), config);
    },
    invocation: () => [
      "exec",
      "--json",
      "--ephemeral",
      "--dangerously-bypass-hook-trust",
      "--dangerously-bypass-approvals-and-sandbox",
      "--cd",
      workspace,
      prompt,
    ],
    artifactPaths: () => [
      [join(codexHome, "hooks.json"), "hooks.json"],
      [join(codexHome, "config.toml"), "config.toml"],
    ],
  };
}

function createOpenCodeAdapter() {
  return {
    binary: join(binaries, "opencode"),
    shellToolNames: ["bash"],
    configure: async () => {
      const config = {
        $schema: "https://opencode.ai/config.json",
        model: "aimock/tapas-ci",
        small_model: "aimock/tapas-ci",
        provider: {
          aimock: {
            npm: "@ai-sdk/openai-compatible",
            name: "aimock",
            options: {
              baseURL: `${mock.url}/v1`,
              apiKey: "tapas-mock-key",
            },
            models: {
              "tapas-ci": {
                name: "Tapas CI",
                limit: { context: 128000, output: 4096 },
              },
            },
          },
        },
      };
      const opencodeConfig = join(xdgConfigHome, "opencode");
      await mkdir(opencodeConfig, { recursive: true });
      await writeFile(
        join(opencodeConfig, "opencode.json"),
        `${JSON.stringify(config, null, 2)}\n`,
      );
    },
    invocation: () => [
      "run",
      "--format",
      "json",
      "--auto",
      "--title",
      "tapas-harness-e2e",
      "--model",
      "aimock/tapas-ci",
      "--dir",
      workspace,
      prompt,
    ],
    artifactPaths: () => [
      [join(xdgConfigHome, "opencode/plugins/tapas.js"), "tapas.js"],
      [join(xdgConfigHome, "opencode/opencode.json"), "opencode.json"],
    ],
  };
}

async function runTapas(...args) {
  return run(tapas, args, { cwd: workspace, label: `tapas-${args[0].slice(2)}` });
}

async function runHarness(phase) {
  return run(adapter.binary, adapter.invocation(), {
    cwd: workspace,
    label: `${harness}-${phase}`,
  });
}

function assertToolResult(requests, expectation) {
  assert.ok(
    requests.length >= 2,
    `${harness} ${expectation.phase}: aimock journal did not record both model requests`,
  );
  assert.ok(
    observedRequests.length >= 2,
    `${harness} ${expectation.phase}: expected a model request followed by a tool-result request`,
  );
  const toolResultRequest = JSON.stringify(observedRequests.at(-1));
  assert.ok(
    toolResultRequest.includes(expectation.includes),
    `${harness} ${expectation.phase}: expected tool result to contain ${JSON.stringify(expectation.includes)}`,
  );
  for (const excluded of expectation.excludes) {
    assert.ok(
      !toolResultRequest.includes(excluded),
      `${harness} ${expectation.phase}: tool result unexpectedly contained ${JSON.stringify(excluded)}`,
    );
  }
}

async function saveJournal(phase) {
  await Promise.all([
    writeFile(
      join(artifactRoot, `${phase}-journal.json`),
      `${JSON.stringify(mock.getRequests(), null, 2)}\n`,
    ),
    writeFile(
      join(artifactRoot, `${phase}-requests.json`),
      `${JSON.stringify(observedRequests, null, 2)}\n`,
    ),
  ]);
}

async function saveHarnessConfiguration(phase) {
  for (const [source, name] of adapter.artifactPaths()) {
    let contents;
    try {
      contents = await readFile(source);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      contents = Buffer.from("<absent>\n");
    }
    await writeFile(join(artifactRoot, `${phase}-${name}`), contents);
  }
}

async function run(command, args, options = {}) {
  const label = options.label ?? command.split("/").at(-1);
  const result = await new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repository,
      env: environment,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    const timeout = setTimeout(() => {
      child.kill("SIGTERM");
      setTimeout(() => child.kill("SIGKILL"), 2_000).unref();
    }, timeoutMs);
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.on("error", (error) => {
      clearTimeout(timeout);
      rejectPromise(error);
    });
    child.on("close", (code, signal) => {
      clearTimeout(timeout);
      resolvePromise({
        code,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      });
    });
  });

  await Promise.all([
    writeFile(join(artifactRoot, `${label}.stdout.log`), result.stdout),
    writeFile(join(artifactRoot, `${label}.stderr.log`), result.stderr),
  ]);
  assert.equal(
    result.code,
    0,
    `${label} exited with ${result.code ?? result.signal}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`,
  );
  return result;
}
