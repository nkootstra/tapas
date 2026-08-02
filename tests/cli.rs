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
        "--hook-eval codex",
        "--setup claude",
        "--setup codex",
        "--unsetup claude",
        "--unsetup codex",
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
fn unsupported_setup_targets_are_usage_errors() {
    for args in [&["--setup=cursor"][..], &["--unsetup", "opencode"][..]] {
        let output = tapas(args);

        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        let error = String::from_utf8(output.stderr).expect("UTF-8 usage error");
        assert_eq!(
            error,
            "usage: tapas --setup <claude|codex> [--dry-run]\n       tapas --unsetup <claude|codex> [--dry-run]\n"
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
fn process_modes_execute_the_requested_child() {
    let normal = tapas(&["/usr/bin/printf", "normal\\n"]);
    let raw = tapas(&["--raw", "--", "/usr/bin/printf", "raw\\n"]);
    let explained = tapas(&["--explain", "/usr/bin/printf", "explained\\n"]);

    assert!(normal.status.success());
    assert_eq!(normal.stdout, b"normal\n");
    assert!(normal.stderr.is_empty());
    assert!(raw.status.success());
    assert_eq!(raw.stdout, b"raw\n");
    assert!(raw.stderr.is_empty());
    assert!(explained.status.success());
    assert_eq!(explained.stdout, b"explained\n");
    assert!(explained.stderr.starts_with(b"\n(tapas explain:"));
}

#[test]
fn claude_hook_updates_only_simple_supported_commands_without_granting_authority() {
    let eligible = tapas_with_stdin(
        &["--hook-eval", "claude"],
        br#"{"hook_event_name":"PreToolUse","permission_mode":"bypassPermissions","tool_input":{"command":"git status --token=s3cr3t\t--short","description":"secret\nline\u0000","flag":true,"ratio":0.5,"nested":{"items":[1e6,null]}}}"#,
        &[],
    );
    assert!(eligible.status.success());
    let executable = env!("CARGO_BIN_EXE_tapas");
    assert!(std::path::Path::new(executable).is_absolute());
    let expected = format!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\"updatedInput\":{{\"command\":\"'{executable}' git status --token=s3cr3t\\t--short\",\"description\":\"secret\\nline\\u0000\",\"flag\":true,\"ratio\":0.5,\"nested\":{{\"items\":[1e6,null]}}}}}}}}\n"
    );
    assert_eq!(eligible.stdout, expected.as_bytes());
    assert!(eligible.stderr.is_empty());
    let prefix = format!("'{executable}'");
    assert_eq!(
        eligible
            .stdout
            .windows(prefix.len())
            .filter(|part| *part == prefix.as_bytes())
            .count(),
        1
    );
    assert!(
        !eligible
            .stdout
            .windows(b"permission".len())
            .any(|part| part == b"permission")
    );

    let already_wrapped =
        format!("{{\"tool_input\":{{\"command\":\"'{executable}' git status\"}}}}");
    for input in [
        br#"{"tool_input":{"command":"git status | cat"}}"#.as_slice(),
        br#"{"tool_input":{"command":"unknown command"}}"#,
        b"invalid JSON",
        already_wrapped.as_bytes(),
    ] {
        let ignored = tapas_with_stdin(&["--hook-eval", "claude"], input, &[]);
        assert!(ignored.status.success());
        assert!(ignored.stdout.is_empty());
        assert!(ignored.stderr.is_empty());
    }

    let self_check = tapas(&["--hook-eval", "claude", "--self-check"]);
    assert!(self_check.status.success());
    assert!(self_check.stdout.is_empty());
    assert!(self_check.stderr.is_empty());
}

#[test]
fn codex_hook_allows_only_the_rewritten_command() {
    let eligible = tapas_with_stdin(
        &["--hook-eval", "codex"],
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git status --short","description":"working tree"}}"#,
        &[],
    );

    assert!(eligible.status.success());
    let executable = env!("CARGO_BIN_EXE_tapas");
    let expected = format!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"allow\",\"updatedInput\":{{\"command\":\"'{executable}' git status --short\",\"description\":\"working tree\"}}}}}}\n"
    );
    assert_eq!(eligible.stdout, expected.as_bytes());
    assert!(eligible.stderr.is_empty());

    for input in [
        br#"{"tool_name":"Bash","tool_input":{"command":"git status"}}"#.as_slice(),
        br#"{"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"git status"}}"#,
        br#"{"hook_event_name":"PreToolUse","tool_input":{"command":"git status"}}"#,
        br#"{"hook_event_name":"PreToolUse","tool_name":"Shell","tool_input":{"command":"git status"}}"#,
    ] {
        let ignored = tapas_with_stdin(&["--hook-eval", "codex"], input, &[]);
        assert!(ignored.status.success());
        assert!(ignored.stdout.is_empty(), "input: {input:?}");
        assert!(ignored.stderr.is_empty(), "input: {input:?}");
    }

    let already_wrapped = format!(
        "{{\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\",\"tool_input\":{{\"command\":\"'{executable}' git status\"}}}}"
    );
    let oversized = format!(
        "{{\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\",\"tool_input\":{{\"command\":\"git status\",\"description\":\"{}\"}}}}",
        "x".repeat(64 * 1024)
    );
    for input in [
        b"invalid JSON".to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git status | cat"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git status\nrm -rf x"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"unknown command"}}"#.to_vec(),
        already_wrapped.into_bytes(),
        oversized.into_bytes(),
    ] {
        let ignored = tapas_with_stdin(&["--hook-eval", "codex"], &input, &[]);
        assert!(ignored.status.success());
        assert!(ignored.stdout.is_empty(), "input length: {}", input.len());
        assert!(ignored.stderr.is_empty(), "input length: {}", input.len());
    }

    let self_check = tapas(&["--hook-eval", "codex", "--self-check"]);
    assert!(self_check.status.success());
    assert!(self_check.stdout.is_empty());
    assert!(self_check.stderr.is_empty());
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
fn stream_environment_names_do_not_disrupt_non_streaming_execution() {
    let baseline = tapas_with_stdin(&["/usr/bin/printf", "ok\\n"], b"", &[]);
    let configured = tapas_with_stdin(
        &["/usr/bin/printf", "ok\\n"],
        b"",
        &[("TAPAS_STREAM", "1"), ("SMLL_STREAM", "0")],
    );

    assert!(configured.status.success());
    assert_eq!(configured.stdout, baseline.stdout);
    assert_eq!(configured.stderr, baseline.stderr);
}
