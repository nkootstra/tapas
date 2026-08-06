use super::*;

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
