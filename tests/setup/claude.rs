use super::*;

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
