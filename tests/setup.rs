#![cfg(unix)]

#[cfg(target_os = "linux")]
use std::ffi::OsString;
use std::fs;
use std::io::{self, Cursor, Write};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_HOME: AtomicU64 = AtomicU64::new(0);

struct TestHome(PathBuf);

impl TestHome {
    fn new() -> Self {
        let sequence = NEXT_HOME.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "tapas-setup-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, suffix: &str) -> PathBuf {
        self.0.join(suffix)
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn tapas(home: &TestHome, args: &[&str], stdin: &[u8]) -> Output {
    tapas_with_env(home, args, stdin, &[])
}

fn tapas_with_env(
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

#[test]
fn legacy_public_setup_entry_points_remain_claude_compatible() {
    let _: fn(tapas::setup::Action, bool, &mut dyn Write, &mut dyn Write) -> io::Result<i32> =
        tapas::setup::configure;

    let mut input = Cursor::new(br#"{"tool_input":{"command":"git status"}}"#);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = tapas::setup::hook_eval(&mut input, &mut stdout, &mut stderr, false).unwrap();

    assert_eq!(code, 0);
    assert!(stderr.is_empty());
    assert!(
        stdout
            .windows(b"\"updatedInput\"".len())
            .any(|part| part == b"\"updatedInput\"")
    );
    assert!(
        !stdout
            .windows(b"permissionDecision".len())
            .any(|part| part == b"permissionDecision")
    );
}

#[test]
fn setup_is_atomic_idempotent_private_and_preserves_unrelated_content() {
    let home = TestHome::new();
    fs::create_dir_all(home.path(".claude")).unwrap();
    let settings = home.path(".claude/settings.json");
    let original = concat!(
        r#"{"theme":"dark","ratio":0.5,"scale":1e6,"large":9223372036854775808,"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"other-hook"}]}]}}"#,
        "\n"
    )
    .as_bytes();
    fs::write(&settings, original).unwrap();

    let dry_run = tapas(&home, &["--setup=claude", "--dry-run"], b"");
    assert!(dry_run.status.success());
    assert!(dry_run.stdout.starts_with(b"[dry-run] would update "));
    assert!(
        dry_run
            .stdout
            .ends_with(b"[dry-run] would record tapas hook ownership\n")
    );
    assert_eq!(fs::read(&settings).unwrap(), original);
    assert!(!home.path(".tapas").exists());

    let setup = tapas(&home, &["--setup", "claude"], b"");
    assert!(setup.status.success(), "{:?}", setup.stderr);
    assert!(setup.stdout.starts_with(b"updated "));
    assert!(setup.stdout.ends_with(b"ok\n"));
    let configured = fs::read(&settings).unwrap();
    assert!(
        configured
            .windows(b"\"theme\":\"dark\"".len())
            .any(|part| part == b"\"theme\":\"dark\"")
    );
    assert!(
        configured
            .windows(b"other-hook".len())
            .any(|part| part == b"other-hook")
    );
    for number in [
        b"\"ratio\":0.5".as_slice(),
        b"\"scale\":1e6",
        b"\"large\":9223372036854775808",
    ] {
        assert!(
            configured.windows(number.len()).any(|part| part == number),
            "missing unchanged number lexeme {number:?}"
        );
    }
    assert!(
        configured
            .windows(b" --hook-eval claude".len())
            .any(|part| part == b" --hook-eval claude")
    );
    assert_eq!(
        configured
            .windows(b"--hook-eval claude".len())
            .filter(|part| *part == b"--hook-eval claude")
            .count(),
        1
    );

    let ownership = home.path(".tapas/setup/claude.owned");
    assert_eq!(
        fs::metadata(home.path(".tapas"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(home.path(".tapas/setup"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&ownership).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::read(home.path(".claude/settings.json.bak.tapas")).unwrap(),
        original
    );

    let repeated = tapas(&home, &["--setup", "claude"], b"");
    assert!(repeated.status.success());
    assert_eq!(repeated.stdout, b"already installed\nok\n");
    assert_eq!(fs::read(&settings).unwrap(), configured);

    let unsetup = tapas(&home, &["--unsetup", "claude"], b"");
    assert!(unsetup.status.success(), "{:?}", unsetup.stderr);
    assert!(unsetup.stdout.starts_with(b"updated "));
    assert!(unsetup.stdout.ends_with(b"ok\n"));
    let restored = fs::read(&settings).unwrap();
    assert!(
        restored
            .windows(b"other-hook".len())
            .any(|part| part == b"other-hook")
    );
    assert!(
        !restored
            .windows(b"--hook-eval claude".len())
            .any(|part| part == b"--hook-eval claude")
    );
    assert!(!ownership.exists());
}

#[test]
fn claude_setup_preserves_every_preexisting_byte_outside_the_inserted_entry() {
    let home = TestHome::new();
    fs::create_dir_all(home.path(".claude")).unwrap();
    let settings = home.path(".claude/settings.json");
    let original = b"{\n  \"escaped\" : \"\\u0074\\/keep\",\n  \"number\": 1.2300e+04,\n  \"hooks\" : { \"PreToolUse\" : [\n    {\"matcher\":\"Bash\",\"hooks\":[{\"type\":\"command\",\"command\":\"other-hook\"}]}\n  ] },\n  \"tail\" : true\n}\n";
    fs::write(&settings, original).unwrap();

    let setup = tapas(&home, &["--setup", "claude"], b"");
    assert!(setup.status.success(), "{:?}", setup.stderr);
    let configured = fs::read(&settings).unwrap();
    assert_eq!(
        configured
            .windows(b"--hook-eval claude".len())
            .filter(|part| *part == b"--hook-eval claude")
            .count(),
        1
    );
    assert!(
        configured
            .windows(b"\\u0074\\/keep".len())
            .any(|p| p == b"\\u0074\\/keep")
    );
    assert!(
        configured
            .windows(b"1.2300e+04".len())
            .any(|p| p == b"1.2300e+04")
    );

    let unsetup = tapas(&home, &["--unsetup", "claude"], b"");
    assert!(unsetup.status.success(), "{:?}", unsetup.stderr);
    assert_eq!(fs::read(&settings).unwrap(), original);
}

#[test]
fn setup_blocks_unowned_tapas_looking_hooks() {
    let home = TestHome::new();
    fs::create_dir_all(home.path(".claude")).unwrap();
    let settings = home.path(".claude/settings.json");
    let original = b"{\"hooks\":{\"PreToolUse\":[{\"matcher\":\"Bash\",\"hooks\":[{\"type\":\"command\",\"command\":\"/tmp/tapas --hook-eval claude\",\"timeout\":10}]}]}}\n";
    fs::write(&settings, original).unwrap();

    let output = tapas(&home, &["--setup", "claude"], b"");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("ownership"));
    assert_eq!(fs::read(&settings).unwrap(), original);
}

#[test]
fn opencode_setup_is_idempotent_and_unsetup_removes_only_tapas() {
    let home = TestHome::new();
    let xdg = home.path("xdg");
    let plugin = xdg.join("opencode/plugins/tapas.js");

    let setup = tapas_with_env(
        &home,
        &["--setup", "opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(setup.status.success(), "{:?}", setup.stderr);
    let content = fs::read(&plugin).unwrap();
    assert!(
        content
            .windows(b"tool.execute.before".len())
            .any(|p| p == b"tool.execute.before")
    );
    assert!(
        content
            .windows(b"Bun.spawnSync".len())
            .any(|p| p == b"Bun.spawnSync")
    );
    assert!(
        content
            .windows(b"--hook-eval".len())
            .any(|p| p == b"--hook-eval")
    );
    assert_opencode_plugin_behavior(&plugin);

    let repeated = tapas_with_env(
        &home,
        &["--setup=opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(repeated.status.success());
    assert_eq!(fs::read(&plugin).unwrap(), content);

    let sibling = xdg.join("opencode/plugins/keep.ts");
    fs::write(&sibling, b"export const keep = true;\n").unwrap();
    let unsetup = tapas_with_env(
        &home,
        &["--unsetup", "opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(unsetup.status.success(), "{:?}", unsetup.stderr);
    assert!(!plugin.exists());
    assert_eq!(fs::read(&sibling).unwrap(), b"export const keep = true;\n");
}

#[test]
#[cfg(target_os = "linux")]
fn opencode_setup_rejects_non_utf8_executable_paths_without_writing_files() {
    let home = TestHome::new();
    let executable = home.0.join(OsString::from_vec(b"tapas-\xff".to_vec()));
    fs::hard_link(env!("CARGO_BIN_EXE_tapas"), &executable).unwrap();
    let xdg = home.path("xdg");

    let output = Command::new(&executable)
        .args(["--setup", "opencode"])
        .env_clear()
        .env("HOME", &home.0)
        .env("XDG_CONFIG_HOME", &xdg)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("UTF-8"));
    assert!(!xdg.join("opencode/plugins/tapas.js").exists());
    assert!(!home.path(".tapas/setup/opencode.owned").exists());
}

#[test]
fn opencode_predecessor_requires_force_and_force_removes_only_recognized_files() {
    let home = TestHome::new();
    let xdg = home.path("xdg");
    let plugins = xdg.join("opencode/plugins");
    fs::create_dir_all(&plugins).unwrap();
    let predecessor = plugins.join("rtk.ts");
    let unrelated = plugins.join("notes.ts");
    fs::write(
        &predecessor,
        b"// rtk rewrite\nexport const RtkOpenCodePlugin = async () => ({ \"tool.execute.before\": async () => {} })\n",
    )
    .unwrap();
    fs::write(&unrelated, b"// rtk is mentioned in documentation only\n").unwrap();

    let blocked = tapas_with_env(
        &home,
        &["--setup", "opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert_eq!(blocked.status.code(), Some(1));
    assert!(predecessor.exists());
    assert!(!plugins.join("tapas.js").exists());

    let forced = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(forced.status.success(), "{:?}", forced.stderr);
    assert!(!predecessor.exists());
    assert_eq!(
        fs::read(&unrelated).unwrap(),
        b"// rtk is mentioned in documentation only\n"
    );
    assert!(plugins.join("tapas.js").exists());
}

#[test]
fn opencode_force_refuses_ambiguous_predecessors_and_preserves_unrelated_directories() {
    for symlinked in [false, true] {
        let home = TestHome::new();
        let xdg = home.path("xdg");
        let plugins = xdg.join("opencode/plugins");
        fs::create_dir_all(&plugins).unwrap();
        let predecessor = plugins.join("rtk.ts");
        let target = home.path("unrecognized-rtk.ts");
        fs::write(&target, b"// user-owned file\n").unwrap();
        if symlinked {
            symlink(&target, &predecessor).unwrap();
        } else {
            fs::write(&predecessor, b"// user-owned file\n").unwrap();
        }
        let config = xdg.join("opencode/opencode.json");
        fs::write(&config, b"{\n  \"theme\": \"dark\"\n}\n").unwrap();

        let output = tapas_with_env(
            &home,
            &["--setup", "opencode", "--force"],
            b"",
            &[("XDG_CONFIG_HOME", xdg.as_path())],
        );

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(fs::read(&config).unwrap(), b"{\n  \"theme\": \"dark\"\n}\n");
        assert!(fs::symlink_metadata(&predecessor).is_ok());
        assert!(!plugins.join("tapas.js").exists());
        assert!(!home.path(".tapas/setup/opencode.owned").exists());
    }

    let home = TestHome::new();
    let xdg = home.path("xdg");
    let unrelated = xdg.join("opencode/plugins/smll-proxy");
    fs::create_dir_all(&unrelated).unwrap();
    let output = tapas_with_env(
        &home,
        &["--setup", "opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(unrelated.is_dir());

    let home = TestHome::new();
    let xdg = home.path("xdg");
    let root = xdg.join("opencode");
    fs::create_dir_all(root.join("plugins")).unwrap();
    let target = home.path("opencode.json");
    fs::write(&target, b"{\"plugin\":[]}\n").unwrap();
    symlink(&target, root.join("opencode.json")).unwrap();
    fs::write(
        root.join("plugins/rtk.ts"),
        b"// rtk rewrite\nexport const RtkOpenCodePlugin = async () => ({ \"tool.execute.before\": async () => {} })\n",
    )
    .unwrap();
    let output = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        fs::symlink_metadata(root.join("opencode.json"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(target).unwrap(), b"{\"plugin\":[]}\n");
    assert!(root.join("plugins/rtk.ts").exists());
    assert!(!root.join("plugins/tapas.js").exists());
}

#[test]
fn opencode_force_removes_smll_registration_losslessly() {
    let home = TestHome::new();
    let xdg = home.path("xdg");
    let root = xdg.join("opencode");
    let proxy = root.join("plugins/smll-proxy");
    fs::create_dir_all(&proxy).unwrap();
    fs::write(
        proxy.join("index.ts"),
        b"// smll-proxy\nexport const SmllProxyPlugin = async () => ({ \"tool.execute.before\": async () => {} });\n",
    )
    .unwrap();
    fs::write(
        proxy.join("package.json"),
        b"{\"name\":\"smll-proxy\",\"main\":\"index.ts\"}\n",
    )
    .unwrap();
    let smll_ownership = home.path(".smll/setup/opencode.owned");
    fs::create_dir_all(smll_ownership.parent().unwrap()).unwrap();
    fs::write(
        &smll_ownership,
        b"smll-setup-v1\nee10f25e7743059b\n265e0e6e87cc138a\n90fcd6f4857a96d7",
    )
    .unwrap();
    let registered = proxy.as_os_str().as_encoded_bytes();
    let mut config =
        b"{\n  \"theme\" : \"dark\",\n  \"plugin\" : [\n    \"keep-me\",\n    \"".to_vec();
    config.extend_from_slice(registered);
    config.extend_from_slice(b"\"\n  ],\n  \"tail\": 1.00e+2\n}\n");
    fs::write(root.join("opencode.json"), &config).unwrap();

    let output = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(output.status.success(), "{:?}", output.stderr);
    let cleaned = fs::read(root.join("opencode.json")).unwrap();
    assert!(
        cleaned
            .windows(b"\"keep-me\"".len())
            .any(|part| part == b"\"keep-me\"")
    );
    assert!(
        cleaned
            .windows(b"1.00e+2".len())
            .any(|part| part == b"1.00e+2")
    );
    assert!(
        !cleaned
            .windows(registered.len())
            .any(|part| part == registered)
    );
    assert!(!proxy.exists());
    assert!(!smll_ownership.exists());
    assert!(root.join("plugins/tapas.js").exists());
}

#[test]
fn opencode_force_preserves_jsonc_and_external_predecessor_conflicts() {
    let home = TestHome::new();
    let xdg = home.path("xdg");
    let root = xdg.join("opencode");
    fs::create_dir_all(&root).unwrap();
    let jsonc = root.join("opencode.jsonc");
    let jsonc_before = b"{ // user comments\n  \"plugin\": [\"smll-proxy\"]\n}\n";
    fs::write(&jsonc, jsonc_before).unwrap();

    let blocked = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(fs::read(&jsonc).unwrap(), jsonc_before);
    assert!(!root.join("plugins/tapas.js").exists());
    assert!(!home.path(".tapas/setup/opencode.owned").exists());

    fs::remove_file(&jsonc).unwrap();
    let custom = home.path("custom-opencode");
    let external = custom.join("plugins/rtk.ts");
    fs::create_dir_all(external.parent().unwrap()).unwrap();
    let external_before = b"// rtk rewrite\nexport const RtkOpenCodePlugin = async () => ({ \"tool.execute.before\": async () => {} })\n";
    fs::write(&external, external_before).unwrap();

    let blocked = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[
            ("XDG_CONFIG_HOME", xdg.as_path()),
            ("OPENCODE_CONFIG_DIR", custom.as_path()),
        ],
    );
    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(fs::read(&external).unwrap(), external_before);
    assert!(!root.join("plugins/tapas.js").exists());
    assert!(!home.path(".tapas/setup/opencode.owned").exists());
}

#[test]
fn codex_setup_manages_its_own_hook_file_and_ownership() {
    let home = TestHome::new();
    fs::create_dir_all(home.path(".codex")).unwrap();
    let hooks = home.path(".codex/hooks.json");
    let original = concat!(
        r#"{"theme":"dark","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"other-hook"}]}]}}"#,
        "\n"
    )
    .as_bytes();
    fs::write(&hooks, original).unwrap();
    fs::set_permissions(&hooks, fs::Permissions::from_mode(0o600)).unwrap();

    let dry_run = tapas(&home, &["--setup=codex", "--dry-run"], b"");
    assert!(dry_run.status.success());
    assert!(dry_run.stdout.starts_with(b"[dry-run] would update "));
    assert_eq!(fs::read(&hooks).unwrap(), original);
    assert!(!home.path(".codex/hooks.json.bak.tapas").exists());
    assert!(!home.path(".tapas/setup/codex.owned").exists());

    let setup = tapas(&home, &["--setup", "codex"], b"");
    assert!(setup.status.success(), "{:?}", setup.stderr);
    assert!(setup.stdout.starts_with(b"updated "));
    assert!(setup.stdout.ends_with(b"ok\n"));
    let configured = fs::read(&hooks).unwrap();
    assert!(
        configured
            .windows(b"other-hook".len())
            .any(|part| part == b"other-hook")
    );
    assert_eq!(
        configured
            .windows(b"--hook-eval codex".len())
            .filter(|part| *part == b"--hook-eval codex")
            .count(),
        1
    );
    assert_eq!(
        fs::metadata(&hooks).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::read(home.path(".codex/hooks.json.bak.tapas")).unwrap(),
        original
    );

    let ownership = home.path(".tapas/setup/codex.owned");
    assert_eq!(
        fs::metadata(&ownership).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let repeated = tapas(&home, &["--setup=codex"], b"");
    assert!(repeated.status.success());
    assert_eq!(repeated.stdout, b"already installed\nok\n");

    let dry_unsetup = tapas(&home, &["--unsetup", "codex", "--dry-run"], b"");
    assert!(dry_unsetup.status.success());
    assert!(dry_unsetup.stdout.starts_with(b"[dry-run] would update "));
    assert_eq!(fs::read(&hooks).unwrap(), configured);
    assert!(ownership.exists());

    let unsetup = tapas(&home, &["--unsetup=codex"], b"");
    assert!(unsetup.status.success(), "{:?}", unsetup.stderr);
    let restored = fs::read(&hooks).unwrap();
    assert!(
        restored
            .windows(b"other-hook".len())
            .any(|part| part == b"other-hook")
    );
    assert!(
        !restored
            .windows(b"--hook-eval codex".len())
            .any(|part| part == b"--hook-eval codex")
    );
    assert!(!ownership.exists());
}

#[test]
fn unsetup_preserves_unrelated_edits_made_after_setup() {
    for (target, config_suffix) in [
        ("claude", ".claude/settings.json"),
        ("codex", ".codex/hooks.json"),
    ] {
        let home = TestHome::new();
        let setup = tapas(&home, &["--setup", target], b"");
        assert!(
            setup.status.success(),
            "target {target}: {:?}",
            setup.stderr
        );
        let config = home.path(config_suffix);
        let edited = insert_before_root_close(
            &fs::read(&config).unwrap(),
            b",\n  \"user_note\": \"keep me\"\n",
        );
        fs::write(&config, &edited).unwrap();

        let unsetup = tapas(&home, &["--unsetup", target], b"");

        assert!(
            unsetup.status.success(),
            "target {target}: {:?}",
            unsetup.stderr
        );
        let restored = fs::read(&config).unwrap();
        assert!(
            restored
                .windows(b"keep me".len())
                .any(|part| part == b"keep me")
        );
        assert!(
            !restored
                .windows(b"--hook-eval".len())
                .any(|part| part == b"--hook-eval")
        );
    }
}

#[test]
fn legacy_v2_ownership_migrates_with_a_hook_free_restoration_backup() {
    let home = TestHome::new();
    fs::create_dir_all(home.path(".claude")).unwrap();
    let settings = home.path(".claude/settings.json");
    let original = b"{\n  \"theme\": \"dark\"\n}\n";
    fs::write(&settings, original).unwrap();
    let setup = tapas(&home, &["--setup", "claude"], b"");
    assert!(setup.status.success(), "{:?}", setup.stderr);
    let configured = fs::read(&settings).unwrap();
    let command_start = configured
        .windows(b"\"command\":\"".len())
        .position(|part| part == b"\"command\":\"")
        .unwrap()
        + b"\"command\":\"".len();
    let command_end = configured[command_start..]
        .windows(b"\",\"timeout\"".len())
        .position(|part| part == b"\",\"timeout\"")
        .unwrap()
        + command_start;
    let mut payload =
        b"{\"matcher\":\"Bash\",\"hooks\":[{\"type\":\"command\",\"command\":\"".to_vec();
    payload.extend_from_slice(&configured[command_start..command_end]);
    payload.extend_from_slice(b"\",\"timeout\":10}]}");
    let mut legacy = b"tapas-setup-v2\n".to_vec();
    legacy.extend_from_slice(&ownership_digest(&payload));
    legacy.push(b'\n');
    legacy.extend_from_slice(&payload);
    fs::write(home.path(".tapas/setup/claude.owned"), legacy).unwrap();

    let migrate = tapas(&home, &["--setup", "claude"], b"");
    assert!(migrate.status.success(), "{:?}", migrate.stderr);
    let unsetup = tapas(&home, &["--unsetup", "claude"], b"");

    assert!(unsetup.status.success(), "{:?}", unsetup.stderr);
    let restored = fs::read(settings).unwrap();
    assert!(restored.windows(b"dark".len()).any(|part| part == b"dark"));
    assert!(
        !restored
            .windows(b"--hook-eval".len())
            .any(|part| part == b"--hook-eval")
    );
}

#[test]
fn claude_and_codex_installations_are_removed_independently() {
    let home = TestHome::new();

    let claude_setup = tapas(&home, &["--setup", "claude"], b"");
    assert!(claude_setup.status.success(), "{:?}", claude_setup.stderr);
    let claude_configured = fs::read(home.path(".claude/settings.json")).unwrap();

    let codex_setup = tapas(&home, &["--setup", "codex"], b"");
    assert!(codex_setup.status.success(), "{:?}", codex_setup.stderr);
    assert!(home.path(".tapas/setup/claude.owned").exists());
    assert!(home.path(".tapas/setup/codex.owned").exists());

    let codex_unsetup = tapas(&home, &["--unsetup", "codex"], b"");
    assert!(codex_unsetup.status.success(), "{:?}", codex_unsetup.stderr);
    assert_eq!(
        fs::read(home.path(".claude/settings.json")).unwrap(),
        claude_configured
    );
    assert!(home.path(".tapas/setup/claude.owned").exists());
    assert!(!home.path(".tapas/setup/codex.owned").exists());

    let claude_unsetup = tapas(&home, &["--unsetup", "claude"], b"");
    assert!(
        claude_unsetup.status.success(),
        "{:?}",
        claude_unsetup.stderr
    );
    assert!(!home.path(".tapas/setup/claude.owned").exists());
}

#[test]
fn empty_codex_home_uses_the_default_client_home() {
    let home = TestHome::new();
    let empty = std::path::Path::new("");

    let setup = tapas_with_env(&home, &["--setup", "codex"], b"", &[("CODEX_HOME", empty)]);

    assert!(setup.status.success(), "{:?}", setup.stderr);
    assert!(home.path(".codex/hooks.json").exists());
    assert!(home.path(".tapas/setup/codex.owned").exists());
}

#[test]
fn codex_setup_and_unsetup_use_the_same_custom_client_home() {
    let home = TestHome::new();
    let codex_home = home.path("custom-codex");
    fs::create_dir_all(&codex_home).unwrap();
    let hooks = codex_home.join("hooks.json");
    let original = b"{\"theme\":\"dark\"}\n";
    fs::write(&hooks, original).unwrap();

    let setup = tapas_with_env(
        &home,
        &["--setup", "codex"],
        b"",
        &[("CODEX_HOME", codex_home.as_path())],
    );
    assert!(setup.status.success(), "{:?}", setup.stderr);
    assert!(
        fs::read(&hooks)
            .unwrap()
            .windows(b"--hook-eval codex".len())
            .any(|part| part == b"--hook-eval codex")
    );
    assert!(!home.path(".codex").exists());

    let unsetup = tapas_with_env(
        &home,
        &["--unsetup", "codex"],
        b"",
        &[("CODEX_HOME", codex_home.as_path())],
    );
    assert!(unsetup.status.success(), "{:?}", unsetup.stderr);
    let restored = fs::read(&hooks).unwrap();
    assert!(
        restored
            .windows(b"\"theme\":\"dark\"".len())
            .any(|part| part == b"\"theme\":\"dark\"")
    );
    assert!(
        !restored
            .windows(b"--hook-eval codex".len())
            .any(|part| part == b"--hook-eval codex")
    );
    assert!(!home.path(".tapas/setup/codex.owned").exists());
}

#[test]
fn codex_ownership_relocates_safely_and_unsetup_uses_the_recorded_path() {
    let home = TestHome::new();
    let first = home.path("codex-one");
    let second = home.path("codex-two");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let first_original = b"{\n  \"profile\": \"one\"\n}\n";
    let second_original = b"{\n  \"profile\": \"two\"\n}\n";
    fs::write(first.join("hooks.json"), first_original).unwrap();
    fs::write(second.join("hooks.json"), second_original).unwrap();

    let setup = tapas_with_env(
        &home,
        &["--setup", "codex"],
        b"",
        &[("CODEX_HOME", first.as_path())],
    );
    assert!(setup.status.success(), "{:?}", setup.stderr);

    let relocate = tapas_with_env(
        &home,
        &["--setup", "codex"],
        b"",
        &[("CODEX_HOME", second.as_path())],
    );
    assert!(relocate.status.success(), "{:?}", relocate.stderr);
    assert_eq!(fs::read(first.join("hooks.json")).unwrap(), first_original);
    assert!(
        fs::read(second.join("hooks.json"))
            .unwrap()
            .windows(b"--hook-eval codex".len())
            .any(|part| part == b"--hook-eval codex")
    );

    let unsetup = tapas_with_env(
        &home,
        &["--unsetup", "codex"],
        b"",
        &[("CODEX_HOME", first.as_path())],
    );
    assert!(unsetup.status.success(), "{:?}", unsetup.stderr);
    assert_eq!(
        fs::read(second.join("hooks.json")).unwrap(),
        second_original
    );
}

#[test]
fn codex_legacy_ownership_does_not_guess_a_changed_client_home() {
    let home = TestHome::new();
    let first = home.path("codex-one");
    let second = home.path("codex-two");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();

    let setup = tapas_with_env(
        &home,
        &["--setup", "codex"],
        b"",
        &[("CODEX_HOME", first.as_path())],
    );
    assert!(setup.status.success(), "{:?}", setup.stderr);
    let configured = fs::read(first.join("hooks.json")).unwrap();
    fs::write(second.join("hooks.json"), &configured).unwrap();

    let command_start = configured
        .windows(b"\"command\":\"".len())
        .position(|part| part == b"\"command\":\"")
        .unwrap()
        + b"\"command\":\"".len();
    let command_end = configured[command_start..]
        .iter()
        .position(|byte| *byte == b'"')
        .unwrap()
        + command_start;
    let mut payload =
        b"{\"matcher\":\"Bash\",\"hooks\":[{\"type\":\"command\",\"command\":\"".to_vec();
    payload.extend_from_slice(&configured[command_start..command_end]);
    payload.extend_from_slice(b"\",\"timeout\":10}]}");
    let mut legacy = b"tapas-setup-v2\n".to_vec();
    legacy.extend_from_slice(&ownership_digest(&payload));
    legacy.push(b'\n');
    legacy.extend_from_slice(&payload);
    let ownership = home.path(".tapas/setup/codex.owned");
    fs::write(&ownership, legacy).unwrap();

    let unsetup = tapas_with_env(
        &home,
        &["--unsetup", "codex"],
        b"",
        &[("CODEX_HOME", second.as_path())],
    );

    assert_eq!(unsetup.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unsetup.stderr).contains("original CODEX_HOME"));
    assert_eq!(fs::read(first.join("hooks.json")).unwrap(), configured);
    assert_eq!(fs::read(second.join("hooks.json")).unwrap(), configured);
    assert!(ownership.exists());
}

#[test]
fn codex_setup_honors_codex_home_and_rejects_modified_state() {
    let home = TestHome::new();
    let codex_home = home.path("custom-codex");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(codex_home.join("hooks.json"), b"{invalid").unwrap();

    let invalid = tapas_with_env(
        &home,
        &["--setup", "codex"],
        b"",
        &[("CODEX_HOME", codex_home.as_path())],
    );
    assert_eq!(invalid.status.code(), Some(1));
    assert_eq!(invalid.stderr, b"hooks.json: invalid JSON\n");
    assert!(!home.path(".codex").exists());

    fs::write(codex_home.join("hooks.json"), b"{}\n").unwrap();
    let setup = tapas_with_env(
        &home,
        &["--setup", "codex"],
        b"",
        &[("CODEX_HOME", codex_home.as_path())],
    );
    assert!(setup.status.success(), "{:?}", setup.stderr);
    let hooks = codex_home.join("hooks.json");
    let modified = replace_once(
        &fs::read(&hooks).unwrap(),
        b"\"matcher\":\"Bash\"",
        b"\"matcher\":\"BashTool\"",
    );
    fs::write(&hooks, &modified).unwrap();

    let repeated = tapas_with_env(
        &home,
        &["--setup=codex"],
        b"",
        &[("CODEX_HOME", codex_home.as_path())],
    );
    assert_eq!(repeated.status.code(), Some(1));
    assert!(repeated.stdout.is_empty());
    assert!(
        repeated
            .stderr
            .starts_with(b"tapas-owned hook entry was modified")
    );
    assert_eq!(fs::read(&hooks).unwrap(), modified);
    assert_eq!(
        modified
            .windows(b"--hook-eval codex".len())
            .filter(|part| *part == b"--hook-eval codex")
            .count(),
        1
    );

    let unsetup = tapas_with_env(
        &home,
        &["--unsetup", "codex"],
        b"",
        &[("CODEX_HOME", codex_home.as_path())],
    );
    assert_eq!(unsetup.status.code(), Some(1));
    assert!(unsetup.stdout.is_empty());
    assert_eq!(
        unsetup.stderr,
        b"tapas-owned hook entry was modified or removed; configuration left untouched; restore the exact owned entry or remove the modified hook and ownership record manually\n"
    );
    assert_eq!(fs::read(&hooks).unwrap(), modified);
    assert!(home.path(".tapas/setup/codex.owned").exists());
}

#[test]
fn setup_refuses_symlinked_configuration_for_each_target() {
    for (target_name, config_suffix) in [
        ("claude", ".claude/settings.json"),
        ("codex", ".codex/hooks.json"),
    ] {
        let home = TestHome::new();
        fs::create_dir_all(home.path(config_suffix).parent().unwrap()).unwrap();
        let symlink_target = home.path("managed-config.json");
        fs::write(&symlink_target, b"{}\n").unwrap();
        symlink(&symlink_target, home.path(config_suffix)).unwrap();

        let output = tapas(&home, &["--setup", target_name], b"");
        assert_eq!(output.status.code(), Some(1), "target: {target_name}");
        assert_eq!(
            output.stderr,
            b"tapas agent setup: symbolic-link configuration is not supported; configuration left untouched\n"
        );
        assert!(
            fs::symlink_metadata(home.path(config_suffix))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&symlink_target).unwrap(), b"{}\n");
    }
}

#[test]
fn unsetup_refuses_symlinked_configuration_for_each_target() {
    for (target_name, config_suffix) in [
        ("claude", ".claude/settings.json"),
        ("codex", ".codex/hooks.json"),
    ] {
        let home = TestHome::new();
        let setup = tapas(&home, &["--setup", target_name], b"");
        assert!(setup.status.success(), "target: {target_name}");

        let config = home.path(config_suffix);
        let link_target = home.path("managed-config.json");
        fs::rename(&config, &link_target).unwrap();
        let target_before = fs::read(&link_target).unwrap();
        symlink(&link_target, &config).unwrap();
        let ownership = home.path(&format!(".tapas/setup/{target_name}.owned"));
        let ownership_before = fs::read(&ownership).unwrap();

        let output = tapas(&home, &["--unsetup", target_name], b"");

        assert_eq!(output.status.code(), Some(1), "target: {target_name}");
        assert_eq!(
            output.stderr,
            b"tapas agent setup: symbolic-link configuration is not supported; configuration left untouched\n"
        );
        assert!(
            fs::symlink_metadata(&config)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&link_target).unwrap(), target_before);
        assert_eq!(fs::read(&ownership).unwrap(), ownership_before);
    }
}

#[test]
fn codex_setup_reports_conflict_sources_and_preserves_unrelated_inline_hooks() {
    let conflicting = TestHome::new();
    fs::create_dir_all(conflicting.path(".codex")).unwrap();
    fs::write(
        conflicting.path(".codex/config.toml"),
        b"[hooks]\ncommand = 'run-toolkit'\n",
    )
    .unwrap();
    let output = tapas(&conflicting, &["--setup=codex"], b"");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stderr,
        b"Conflicting command-wrapper integration detected in config.toml. Remove it first, then run tapas --setup codex again.\n"
    );
    assert!(!conflicting.path(".codex/hooks.json").exists());

    let hooks_conflict = TestHome::new();
    fs::create_dir_all(hooks_conflict.path(".codex")).unwrap();
    let hooks_content = b"{\"plugin\":\"run-toolkit\"}\n";
    fs::write(hooks_conflict.path(".codex/hooks.json"), hooks_content).unwrap();
    let output = tapas(&hooks_conflict, &["--setup", "codex"], b"");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stderr,
        b"Conflicting command-wrapper integration detected in hooks.json. Remove it first, then run tapas --setup codex again.\n"
    );
    assert_eq!(
        fs::read(hooks_conflict.path(".codex/hooks.json")).unwrap(),
        hooks_content
    );

    let unrelated = TestHome::new();
    fs::create_dir_all(unrelated.path(".codex")).unwrap();
    let inline_content = b"[hooks]\ncommand = 'other-hook'\n";
    fs::write(unrelated.path(".codex/config.toml"), inline_content).unwrap();
    let output = tapas(&unrelated, &["--setup", "codex"], b"");
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(
        fs::read(unrelated.path(".codex/config.toml")).unwrap(),
        inline_content
    );
    assert!(unrelated.path(".codex/hooks.json").exists());
}

#[test]
fn setup_refuses_invalid_conflicting_or_user_modified_state() {
    let invalid = TestHome::new();
    fs::create_dir_all(invalid.path(".claude")).unwrap();
    fs::write(invalid.path(".claude/settings.json"), b"{invalid").unwrap();
    let output = tapas(&invalid, &["--setup", "claude"], b"");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stderr, b"settings.json: invalid JSON\n");
    assert_eq!(
        fs::read(invalid.path(".claude/settings.json")).unwrap(),
        b"{invalid"
    );

    let conflict = TestHome::new();
    fs::create_dir_all(conflict.path(".claude")).unwrap();
    let content = concat!(r#"{"plugin":"run-toolkit"}"#, "\n").as_bytes();
    fs::write(conflict.path(".claude/settings.json"), content).unwrap();
    let output = tapas(&conflict, &["--setup", "claude"], b"");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output
            .stderr
            .starts_with(b"Conflicting command-wrapper integration")
    );
    assert_eq!(
        fs::read(conflict.path(".claude/settings.json")).unwrap(),
        content
    );

    for (original, replacement) in [
        (
            b"\"type\":\"command\"".as_slice(),
            b"\"type\":\"prompt\"".as_slice(),
        ),
        (b"\"matcher\":\"Bash\"", b"\"matcher\":\"BashTool\""),
        (b"\"timeout\":10", b"\"timeout\":20"),
        (
            b"\"timeout\":10",
            b"\"timeout\":10,\"statusMessage\":\"keep me\"",
        ),
    ] {
        let modified = TestHome::new();
        let setup = tapas(&modified, &["--setup", "claude"], b"");
        assert!(setup.status.success());
        let settings = modified.path(".claude/settings.json");
        let content = replace_once(&fs::read(&settings).unwrap(), original, replacement);
        fs::write(&settings, &content).unwrap();

        let output = tapas(&modified, &["--unsetup=claude"], b"");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"tapas-owned hook entry was modified or removed; configuration left untouched; restore the exact owned entry or remove the modified hook and ownership record manually\n",
            "replacement {replacement:?}",
        );
        assert_eq!(fs::read(&settings).unwrap(), content);
        assert!(modified.path(".tapas/setup/claude.owned").exists());
    }
}

#[test]
fn setup_rejects_ambiguous_json_and_ignores_unrelated_predecessor_text() {
    for content in [
        b"\n  \t".as_slice(),
        b"\xef\xbb\xbf{}\n",
        b"{\"theme\":\"one\",\"theme\":\"two\"}\n",
    ] {
        let home = TestHome::new();
        fs::create_dir_all(home.path(".claude")).unwrap();
        let settings = home.path(".claude/settings.json");
        fs::write(&settings, content).unwrap();
        let output = tapas(&home, &["--setup", "claude"], b"");
        assert_eq!(output.status.code(), Some(1), "content: {content:?}");
        assert_eq!(fs::read(settings).unwrap(), content);
    }

    let home = TestHome::new();
    fs::create_dir_all(home.path(".claude")).unwrap();
    let settings = home.path(".claude/settings.json");
    let content = b"{\"notes\":\"rtk and smll are migration history\",\"theme\":\"dark\"}\n";
    fs::write(&settings, content).unwrap();
    let output = tapas(&home, &["--setup", "claude"], b"");
    assert!(output.status.success(), "{:?}", output.stderr);
    let configured = fs::read(settings).unwrap();
    assert!(
        configured
            .windows(content.len() - 2)
            .any(|part| part == &content[..content.len() - 2])
    );
}

fn replace_once(input: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
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

fn insert_before_root_close(input: &[u8], insertion: &[u8]) -> Vec<u8> {
    let end = input.iter().rposition(|byte| *byte == b'}').unwrap();
    let mut output = Vec::with_capacity(input.len() + insertion.len());
    output.extend_from_slice(&input[..end]);
    output.extend_from_slice(insertion);
    output.extend_from_slice(&input[end..]);
    output
}

fn ownership_digest(input: &[u8]) -> [u8; 16] {
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

fn assert_opencode_plugin_behavior(plugin: &std::path::Path) {
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
  if (!new TextDecoder().decode(options.stdin).includes('"command":"git status"')) throw new Error("bad stdin");
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
