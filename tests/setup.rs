#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapas"));
    command
        .args(args)
        .env_clear()
        .env("HOME", &home.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
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
        assert!(
            output
                .stderr
                .starts_with(b"tapas-owned hook entry was modified"),
            "replacement {replacement:?}: {:?}",
            output.stderr
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
