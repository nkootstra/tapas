#![cfg(unix)]

use std::fs;
use std::io::{self, Cursor, Write};
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
    assert!(unsetup.status.success());
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

        assert!(output.status.success());
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
