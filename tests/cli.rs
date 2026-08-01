use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn tapas(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tapas"))
        .args(args)
        .output()
        .expect("run tapas")
}

fn tapas_with_stdin(args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapas"));
    command
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn tapas");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write tapas stdin");
    child.wait_with_output().expect("wait for tapas")
}

#[test]
fn version_identifies_the_new_tapas_lineage() {
    let output = tapas(&["--version"]);

    assert!(output.status.success());
    assert_eq!(output.stdout, b"tapas 0.1.0\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn help_exposes_only_the_milestone_cli_surface() {
    let output = tapas(&["--help"]);

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    for option in [
        "--filters",
        "--raw",
        "--explain",
        "--rewrite",
        "--hook-eval claude",
        "--setup claude",
        "--unsetup claude",
    ] {
        assert!(help.contains(option), "missing {option:?} in {help:?}");
    }
    for deferred in ["smll", "--stats", "--discover", "--err", "--test"] {
        assert!(
            !help.contains(deferred),
            "unexpected {deferred:?} in {help:?}"
        );
    }
    assert!(output.stderr.is_empty());
}

#[test]
fn filters_report_the_canonical_commands_and_runners() {
    let output = tapas(&["--filters"]);

    assert!(output.status.success());
    let filters = String::from_utf8(output.stdout).expect("UTF-8 filters");
    assert!(filters.starts_with("tapas filters\n"));
    for command in [
        "acli",
        "cargo",
        "docker-compose",
        "git",
        "pre-commit",
        "zig",
    ] {
        assert!(
            filters.contains(command),
            "missing {command:?} in {filters:?}"
        );
    }
    for runner in ["npx", "pnpm exec", "poetry run", "uv run", "uvx"] {
        assert!(
            filters.contains(runner),
            "missing {runner:?} in {filters:?}"
        );
    }
    assert!(!filters.contains("smll"));
    assert!(output.stderr.is_empty());
}

#[test]
fn rewrite_wraps_path_qualified_commands_and_shell_escapes_arguments() {
    let output = tapas(&[
        "--rewrite",
        "/usr/local/bin/git",
        "commit",
        "-m",
        "don't expand $HOME",
    ]);

    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"tapas /usr/local/bin/git commit -m 'don'\\''t expand $HOME'\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn rewrite_leaves_unsupported_and_already_wrapped_commands_unwrapped() {
    let unsupported = tapas(&["--rewrite", "python", "script.py"]);
    let wrapped = tapas(&["--rewrite", "/opt/tapas", "git", "status"]);

    assert!(unsupported.status.success());
    assert_eq!(unsupported.stdout, b"python script.py\n");
    assert!(unsupported.stderr.is_empty());
    assert!(wrapped.status.success());
    assert_eq!(wrapped.stdout, b"/opt/tapas git status\n");
    assert!(wrapped.stderr.is_empty());
}

#[test]
fn deferred_state_modes_are_concise_usage_errors() {
    for args in [
        &["--stats"][..],
        &["--discover"][..],
        &["--err", "git", "status"][..],
        &["--test", "git", "status"][..],
    ] {
        let output = tapas(args);

        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        let error = String::from_utf8(output.stderr).expect("UTF-8 usage error");
        assert!(
            error.starts_with("usage: tapas "),
            "args: {args:?}: {error:?}"
        );
        assert!(error.lines().count() <= 2, "args: {args:?}: {error:?}");
        assert!(!error.contains("placeholder"), "args: {args:?}: {error:?}");
    }
}

#[test]
fn non_claude_setup_targets_are_usage_errors() {
    for args in [
        &["--setup", "codex"][..],
        &["--setup=cursor"][..],
        &["--unsetup", "opencode"][..],
        &["--unsetup=codex"][..],
    ] {
        let output = tapas(args);

        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        let error = String::from_utf8(output.stderr).expect("UTF-8 usage error");
        assert_eq!(
            error,
            "usage: tapas --setup claude [--dry-run]\n       tapas --unsetup claude [--dry-run]\n"
        );
    }
}

#[test]
fn raw_stdin_mode_preserves_arbitrary_bytes() {
    let input = b"plain\0binary\xffand\ntext";
    let output = tapas_with_stdin(&["--raw"], input, &[]);

    assert!(output.status.success());
    assert_eq!(output.stdout, input);
    assert!(output.stderr.is_empty());
}

#[test]
fn only_the_tapas_lossless_environment_contract_activates_raw_pipe_mode() {
    let input = "build    complete\n".repeat(300).into_bytes();
    let tapas_env = tapas_with_stdin(&[], &input, &[("TAPAS_LOSSLESS", "1")]);
    let smll_env = tapas_with_stdin(&[], &input, &[("SMLL_LOSSLESS", "1")]);

    assert!(tapas_env.status.success());
    assert_eq!(tapas_env.stdout, input);
    assert!(tapas_env.stderr.is_empty());

    assert!(smll_env.status.success());
    assert_ne!(smll_env.stdout, input);
    assert_eq!(smll_env.stdout, b"build complete \xc3\x97300\n");
    assert!(smll_env.stderr.is_empty());
}

#[test]
fn process_boundaries_never_report_placeholder_success() {
    for args in [
        &["git", "status"][..],
        &["--raw", "--", "git", "status"][..],
        &["--explain", "git", "status"][..],
    ] {
        let output = tapas(args);

        assert_eq!(output.status.code(), Some(70), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert_eq!(
            output.stderr, b"tapas: process execution is not available in the foundation build\n",
            "args: {args:?}"
        );
    }
}

#[test]
fn claude_only_integration_boundaries_are_exposed_but_not_reported_as_complete() {
    for args in [
        &["--hook-eval", "claude"][..],
        &["--setup", "claude"][..],
        &["--unsetup=claude", "--dry-run"][..],
    ] {
        let output = tapas_with_stdin(args, b"{}", &[]);

        assert_eq!(output.status.code(), Some(70), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        let error = String::from_utf8(output.stderr).expect("UTF-8 boundary error");
        assert!(
            error.starts_with("tapas: Claude "),
            "args: {args:?}: {error:?}"
        );
        assert!(!error.starts_with("usage:"), "args: {args:?}: {error:?}");
    }
}

#[test]
fn legacy_stream_and_home_state_have_no_effect() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    let home = std::env::temp_dir().join(format!("tapas-cli-{}-{unique}", std::process::id()));
    fs::create_dir_all(home.join(".smll")).expect("create legacy state directory");
    fs::write(home.join(".smll/config"), b"stream=1\n").expect("write legacy config");
    let home_value = home.to_str().expect("UTF-8 temporary path");

    let baseline = tapas_with_stdin(&["git", "status"], b"", &[]);
    let legacy = tapas_with_stdin(
        &["git", "status"],
        b"",
        &[("HOME", home_value), ("SMLL_STREAM", "1")],
    );

    fs::remove_dir_all(&home).expect("remove temporary home");
    assert_eq!(legacy.status.code(), baseline.status.code());
    assert_eq!(legacy.stdout, baseline.stdout);
    assert_eq!(legacy.stderr, baseline.stderr);
}

#[test]
fn tapas_stream_is_the_only_active_stream_environment_name() {
    let output = tapas_with_stdin(
        &["git", "status"],
        b"",
        &[("TAPAS_STREAM", "1"), ("SMLL_STREAM", "0")],
    );

    assert_eq!(output.status.code(), Some(70));
    assert_eq!(
        output.stderr,
        b"tapas: streaming process execution is not available in the foundation build\n"
    );
}
