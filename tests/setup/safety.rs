use std::fs;
use std::os::unix::fs::symlink;

use super::support::{TestHome, insert_before_root_close, tapas};

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
