use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use super::common;

pub(crate) struct TestHome(pub(crate) PathBuf);

impl TestHome {
    pub(crate) fn new() -> Self {
        Self::new_in(&std::env::temp_dir())
    }

    pub(crate) fn new_in(parent: &Path) -> Self {
        Self(common::unique_temp_dir(parent, "tapas-setup-test"))
    }

    pub(crate) fn path(&self, suffix: &str) -> PathBuf {
        self.0.join(suffix)
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

pub(crate) fn tapas(home: &TestHome, args: &[&str], stdin: &[u8]) -> Output {
    tapas_with_env(home, args, stdin, &[])
}

pub(crate) fn tapas_with_env(
    home: &TestHome,
    args: &[&str],
    stdin: &[u8],
    environment: &[(&str, &std::path::Path)],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapas"));
    command
        .args(args)
        .env_clear()
        .env("HOME", &home.0)
        .envs(environment.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

pub(crate) fn replace_once(input: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    let start = input
        .windows(needle.len())
        .position(|part| part == needle)
        .expect("replacement target");
    let mut output = Vec::with_capacity(input.len() - needle.len() + replacement.len());
    output.extend_from_slice(&input[..start]);
    output.extend_from_slice(replacement);
    output.extend_from_slice(&input[start + needle.len()..]);
    output
}

pub(crate) fn insert_before_root_close(input: &[u8], insertion: &[u8]) -> Vec<u8> {
    let end = input.iter().rposition(|byte| *byte == b'}').unwrap();
    let mut output = Vec::with_capacity(input.len() + insertion.len());
    output.extend_from_slice(&input[..end]);
    output.extend_from_slice(insertion);
    output.extend_from_slice(&input[end..]);
    output
}

pub(crate) fn ownership_digest(input: &[u8]) -> [u8; 16] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for byte in input {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100_0000_01b3);
    }
    let mut output = [0_u8; 16];
    for index in (0..output.len()).rev() {
        output[index] = HEX[(value & 0x0f) as usize];
        value >>= 4;
    }
    output
}

pub(crate) fn assert_opencode_plugin_behavior(plugin: &std::path::Path) {
    if Command::new("bun").arg("--version").output().is_err() {
        assert!(
            std::env::var_os("TAPAS_REQUIRE_BUN").is_none(),
            "Bun is required for the OpenCode plugin runtime contract"
        );
        return;
    }
    let url = format!("file://{}", plugin.display());
    let script = r#"
const plugin = await import(process.env.TAPAS_PLUGIN_URL);
const hook = (await plugin.Tapas())["tool.execute.before"];
let calls = 0;
Bun.spawnSync = (_argv, options) => {
  calls += 1;
  const stdin = new TextDecoder().decode(options.stdin);
  if (!stdin.includes('"command":"git status"') || !stdin.includes('"cwd":"/work"')) throw new Error("bad stdin");
  return { exitCode: 0, stdout: { toString: () => "'/tmp/tapas' git status\n" } };
};
const other = { args: { command: "git status", workdir: "/work", timeout: 123 } };
await hook({ tool: "read" }, other);
if (calls !== 0 || other.args.command !== "git status") throw new Error("non-bash mutated");
const success = { args: { command: "git status", workdir: "/work", timeout: 123 } };
await hook({ tool: "bash" }, success);
if (success.args.command !== "'/tmp/tapas' git status") throw new Error("rewrite missing");
if (success.args.workdir !== "/work" || success.args.timeout !== 123) throw new Error("other args changed");
Bun.spawnSync = () => ({ exitCode: 1, stdout: { toString: () => "ignored\n" } });
const failed = { args: { command: "git status", workdir: "/work" } };
await hook({ tool: "bash" }, failed);
if (failed.args.command !== "git status") throw new Error("nonzero spawn did not fail open");
Bun.spawnSync = () => { throw new Error("spawn failed"); };
const thrown = { args: { command: "git status", workdir: "/work" } };
await hook({ tool: "bash" }, thrown);
if (thrown.args.command !== "git status") throw new Error("exception did not fail open");
Bun.spawnSync = (_argv, options) => {
  const stdin = new TextDecoder().decode(options.stdin);
  if (stdin.includes('"cwd"')) throw new Error("cwd sent without workdir");
  return { exitCode: 0, stdout: { toString: () => "'/tmp/tapas' git status\n" } };
};
const noWorkdir = { args: { command: "git status" } };
await hook({ tool: "bash" }, noWorkdir);
if (noWorkdir.args.command !== "'/tmp/tapas' git status") throw new Error("static rewrite missing without workdir");
"#;
    let output = Command::new("bun")
        .args(["-e", script])
        .env("TAPAS_PLUGIN_URL", url)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "generated OpenCode plugin failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
