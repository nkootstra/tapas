use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
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
    assert_eq!(
        output.stdout,
        format!("tapas {}\n", env!("TAPAS_BUILD_LABEL")).into_bytes()
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn help_exposes_only_the_milestone_cli_surface() {
    let output = tapas(&["--help"]);

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    for option in [
        "--filters",
        "--completions bash",
        "--raw",
        "--explain",
        "--rewrite",
        "--hook-eval claude",
        "--hook-eval codex",
        "--hook-eval opencode",
        "--setup claude",
        "--setup codex",
        "--setup opencode",
        "--unsetup claude",
        "--unsetup codex",
        "--unsetup opencode",
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
fn invalid_tapas_parameters_explain_the_error_and_show_full_help() {
    for (args, explanation) in [
        (&["--v"][..], "unknown option \"--v\""),
        (&["--help", "extra"][..], "--help does not accept arguments"),
        (
            &["--version", "extra"][..],
            "--version does not accept arguments",
        ),
        (
            &["--filters", "extra"][..],
            "--filters does not accept arguments",
        ),
        (&["--raw", "--"][..], "--raw requires a command after --"),
        (&["--explain"][..], "--explain requires a command"),
        (&["--rewrite"][..], "--rewrite requires a command"),
        (
            &["--hook-eval", "cursor"][..],
            "--hook-eval requires claude, codex, or opencode",
        ),
        (
            &["--setup", "cursor"][..],
            "invalid --setup or --unsetup arguments",
        ),
        (
            &["--setup", "claude", "--force"][..],
            "invalid --setup or --unsetup arguments",
        ),
        (
            &["--completions"][..],
            "--completions requires bash, zsh, or fish",
        ),
        (
            &["--completions", "powershell"][..],
            "--completions requires bash, zsh, or fish",
        ),
    ] {
        let output = tapas(args);

        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        let error = String::from_utf8(output.stderr).expect("UTF-8 usage error");
        assert!(
            error.starts_with(&format!("tapas: {explanation}\n\nUsage:\n")),
            "args: {args:?}: {error:?}"
        );
        assert!(error.contains("  tapas <cmd...>\n"), "args: {args:?}");
        assert!(error.contains("  --completions"), "args: {args:?}");
        assert!(error.contains("  -h, --help"), "args: {args:?}");
    }
}

#[test]
fn invalid_non_utf8_options_remain_actionable() {
    let output = Command::new(env!("CARGO_BIN_EXE_tapas"))
        .arg(OsString::from_vec(vec![b'-', b'-', 0xff]))
        .output()
        .expect("run tapas");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.starts_with(b"tapas: unknown option "));
    assert!(
        output
            .stderr
            .windows(b"\n\nUsage:\n".len())
            .any(|window| window == b"\n\nUsage:\n")
    );
    assert!(!output.stderr.contains(&0xff));
}

#[test]
fn completions_cover_the_tapas_surface_for_supported_shells() {
    for (shell, marker) in [
        ("bash", "complete -F _tapas tapas"),
        ("zsh", "#compdef tapas"),
        ("fish", "complete -c tapas"),
    ] {
        let output = tapas(&["--completions", shell]);

        assert!(output.status.success(), "shell: {shell:?}");
        assert!(output.stderr.is_empty(), "shell: {shell:?}");
        let completion = String::from_utf8(output.stdout).expect("UTF-8 completion");
        assert!(completion.contains(marker), "shell: {shell:?}");
        for candidate in [
            "--help",
            "--version",
            "--filters",
            "--raw",
            "--explain",
            "--rewrite",
            "--hook-eval",
            "--setup",
            "--unsetup",
            "--completions",
            "claude",
            "codex",
            "opencode",
            "bash",
            "zsh",
            "fish",
        ] {
            let candidate = if shell == "fish" {
                candidate.trim_start_matches('-')
            } else {
                candidate
            };
            assert!(
                completion.contains(candidate),
                "missing {candidate:?} in {shell:?} completion"
            );
        }
        for unsupported in ["smll", "rtk", "powershell"] {
            assert!(
                !completion.contains(unsupported),
                "unexpected {unsupported:?} in {shell:?} completion"
            );
        }
    }
}

#[test]
fn help_and_completions_expose_plugin_management_without_changing_filters() {
    let help = tapas(&["--help"]);
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("tapas --plugin resolve [--json] -- <cmd...>"));
    assert!(help.contains("tapas --plugin check -- <absolute-path>"));
    assert!(help.contains("tapas --plugin test <id>"));
    assert!(help.contains("tapas --plugin pin <id> [--sha256 <hex>]"));
    assert!(help.contains("tapas --plugin <unpin|untrust|test> <id>"));
    assert!(help.contains("tapas --plugin list [--json]"));
    assert!(help.contains("--plugin"));
    for shell in ["bash", "zsh", "fish"] {
        let output = tapas(&["--completions", shell]);
        let completion = String::from_utf8(output.stdout).unwrap();
        for value in [
            "--plugin",
            "check",
            "test",
            "trust",
            "bind",
            "resolve",
            "approve-project",
            "list",
        ] {
            assert!(completion.contains(value), "missing {value:?} for {shell}");
        }
    }
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
    for runner in ["bunx", "npx", "pnpm exec", "poetry run", "uv run", "uvx"] {
        assert!(
            filters.contains(runner),
            "missing {runner:?} in {filters:?}"
        );
    }
    for (heading, entries) in [
        (
            "Compact routes:",
            &[
                "pip_install",
                "git_status",
                "cargo_nextest",
                "playwright_test",
                "prisma_migrate",
                "golangci_lint",
                "docker_compose_up",
            ][..],
        ),
        (
            "Exact-output policies:",
            &[
                "machine_output",
                "pip_query_or_machine_exact",
                "grep_output_shaping_exact",
                "docker_machine_exact",
            ][..],
        ),
        (
            "Inherited/stream policies:",
            &[
                "vite_lifecycle_inherit",
                "playwright_interactive_inherit",
                "docker_stats_stream_inherit",
            ][..],
        ),
    ] {
        assert!(
            filters.contains(heading),
            "missing {heading:?} in {filters:?}"
        );
        for entry in entries {
            assert!(filters.contains(entry), "missing {entry:?} in {filters:?}");
        }
    }
    assert!(!filters.contains("smll"));
    assert!(output.stderr.is_empty());
}

#[test]
fn expanded_catalog_reports_and_rewrites_the_supported_commands() {
    assert_eq!(tapas::catalog::AUTO_WRAP_COMMANDS.len(), 76);
    assert_eq!(tapas::catalog::WRAPPER_COMMANDS.len(), 85);
    let filters = tapas(&["--filters"]);
    let filters = String::from_utf8(filters.stdout).expect("UTF-8 filters");
    for command in [
        "pip",
        "pip3",
        "uv",
        "uvx",
        "vite",
        "esbuild",
        "cmake",
        "ctest",
        "playwright",
        "helm",
        "grep",
        "bat",
        "batcat",
    ] {
        assert!(
            filters.contains(command),
            "missing {command:?} in {filters:?}"
        );
        let (arguments, expected) = if command == "uvx" {
            (
                vec!["--rewrite", command, "vite", "--version"],
                "tapas uvx vite --version\n".to_owned(),
            )
        } else {
            (
                vec!["--rewrite", command, "--version"],
                format!("tapas {command} --version\n"),
            )
        };
        let rewritten = tapas(&arguments);
        assert_eq!(
            rewritten.stdout,
            expected.as_bytes(),
            "command: {command:?}"
        );
    }
}

#[test]
fn expanded_catalog_commands_rewrite_directly_and_through_transparent_runners() {
    let promoted = [
        "diff",
        "dotnet",
        "golangci-lint",
        "gt",
        "head",
        "mypy",
        "prisma",
        "rake",
        "rspec",
        "rubocop",
        "ruff",
        "tail",
        "wc",
    ];
    let executable = env!("CARGO_BIN_EXE_tapas");

    for target in ["claude", "opencode"] {
        for command in promoted {
            let direct = format!(r#"{{"tool_input":{{"command":"{command} --version"}}}}"#);
            let output = tapas_with_stdin(&["--hook-eval", target], direct.as_bytes(), &[]);
            assert!(
                output.status.success(),
                "target: {target}, command: {command}"
            );
            let stdout = String::from_utf8(output.stdout).expect("UTF-8 hook output");
            assert!(
                stdout.contains(&format!("'{executable}' {command} --version")),
                "target: {target}, command: {command}, output: {stdout:?}",
            );
            assert_eq!(stdout.matches(executable).count(), 1);
        }

        for command in [
            "npx gt status",
            "uv run rubocop",
            "pnpm exec prisma migrate status",
        ] {
            let input = format!(r#"{{"tool_input":{{"command":{command:?}}}}}"#);
            let output = tapas_with_stdin(&["--hook-eval", target], input.as_bytes(), &[]);
            assert!(
                output.status.success(),
                "target: {target}, command: {command}"
            );
            let stdout = String::from_utf8(output.stdout).expect("UTF-8 hook output");
            assert!(
                stdout.contains(&format!("'{executable}' {command}")),
                "target: {target}, command: {command}, output: {stdout:?}",
            );
            assert_eq!(stdout.matches(executable).count(), 1);
        }
    }
}

#[test]
fn all_auto_wrap_commands_rewrite_directly() {
    for command in tapas::catalog::AUTO_WRAP_COMMANDS {
        let arguments = if matches!(*command, "bunx" | "uvx") {
            vec!["--rewrite", command, "vite", "--version"]
        } else if *command == "pnpm" {
            vec!["--rewrite", command, "install"]
        } else {
            vec!["--rewrite", command, "--version"]
        };
        let output = tapas(&arguments);
        assert!(output.status.success(), "command: {command:?}");
        assert!(
            output
                .stdout
                .starts_with(format!("tapas {command} ").as_bytes()),
            "command: {command:?}, output: {:?}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn cargo_check_and_clippy_routes_compact_recognized_build_output() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "tapas-cli-cargo-routes-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("create fake cargo directory");
    let cargo = directory.join("cargo");
    fs::write(
        &cargo,
        b"#!/bin/sh\nprintf '   Compiling demo v0.1.0\\n    Finished dev [unoptimized] target(s) in 0.1s\\n'\n",
    )
    .expect("write fake cargo");
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).expect("make cargo executable");

    for subcommand in ["check", "clippy"] {
        let output = Command::new(env!("CARGO_BIN_EXE_tapas"))
            .arg(&cargo)
            .arg(subcommand)
            .env_remove("TAPAS_LOSSLESS")
            .env_remove("TAPAS_STREAM")
            .env_remove("TAPAS_RAW")
            .output()
            .expect("run fake cargo through tapas");
        assert!(output.status.success(), "subcommand: {subcommand}");
        assert_eq!(
            output.stdout, b"cargo: Finished dev; 1 crates\n",
            "subcommand: {subcommand}",
        );
        assert!(output.stderr.is_empty(), "subcommand: {subcommand}");
    }

    fs::remove_dir_all(directory).expect("remove fake cargo directory");
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
fn rewrite_and_non_codex_hooks_follow_unambiguous_runner_chains() {
    let rewritten = tapas(&[
        "--rewrite",
        "npx",
        "uvx",
        "bunx",
        "uv",
        "run",
        "vite",
        "dev",
    ]);
    assert_eq!(rewritten.stdout, b"tapas npx uvx bunx uv run vite dev\n");

    for direct in [
        &["--rewrite", "pnpm", "--version"][..],
        &["--rewrite", "pnpm", "--filter", "app", "test"][..],
    ] {
        let output = tapas(direct);
        assert!(
            output.stdout.starts_with(b"tapas pnpm "),
            "args: {direct:?}"
        );
    }

    let pnpm_exec = tapas(&[
        "--rewrite",
        "pnpm",
        "--filter",
        "app",
        "exec",
        "vite",
        "dev",
    ]);
    assert_eq!(pnpm_exec.stdout, b"tapas pnpm --filter app exec vite dev\n");

    for args in [
        &["--rewrite", "npx", "unknown-tool"][..],
        &["--rewrite", "npx", "--future", "vite", "dev"][..],
        &["--rewrite", "pnpm", "exec", "--future", "vite"][..],
        &["--rewrite", "pnpm", "--future", "exec", "vite"][..],
        &[
            "--rewrite",
            "npx",
            "uvx",
            "bunx",
            "uv",
            "run",
            "npx",
            "vite",
        ][..],
    ] {
        let output = tapas(args);
        assert!(!output.stdout.starts_with(b"tapas "), "args: {args:?}");
    }

    for target in ["claude", "opencode"] {
        let output = tapas_with_stdin(
            &["--hook-eval", target],
            br#"{"tool_input":{"command":"npx vite dev"}}"#,
            &[],
        );
        assert!(output.status.success());
        assert!(
            output
                .stdout
                .windows(b"npx vite dev".len())
                .any(|part| part == b"npx vite dev")
        );
    }

    for command in ["npx git status", "npx vite dev", "grep needle file"] {
        let input = format!(
            "{{\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\",\"cwd\":\"/tmp\",\"tool_input\":{{\"command\":{command:?}}}}}"
        );
        let output = tapas_with_stdin(
            &["--hook-eval", "codex"],
            input.as_bytes(),
            &[("PATH", "/usr/bin:/bin")],
        );
        assert!(output.status.success());
        assert!(output.stdout.is_empty(), "command: {command:?}");
    }
}

#[test]
fn deferred_state_modes_are_actionable_usage_errors() {
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
        assert!(error.starts_with("tapas: "), "args: {args:?}: {error:?}");
        assert!(error.contains("\n\nUsage:\n"), "args: {args:?}: {error:?}");
        assert!(!error.contains("placeholder"), "args: {args:?}: {error:?}");
    }
}

#[test]
fn unsupported_setup_targets_are_usage_errors() {
    for args in [&["--setup=cursor"][..], &["--unsetup", "cursor"][..]] {
        let output = tapas(args);

        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        let error = String::from_utf8(output.stderr).expect("UTF-8 usage error");
        assert!(error.starts_with("tapas: invalid --setup or --unsetup arguments\n\n"));
        assert!(error.contains("Usage:\n"));
    }
}

#[test]
fn force_is_valid_only_for_opencode_setup() {
    for args in [
        &["--setup", "claude", "--force"][..],
        &["--setup", "codex", "--force"][..],
        &["--unsetup", "opencode", "--force"][..],
    ] {
        let output = tapas(args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
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
fn child_options_are_never_reinterpreted_as_tapas_options() {
    let normal = tapas(&["/usr/bin/printf", "%s\\n", "--help"]);
    let raw = tapas(&["--raw", "--", "/usr/bin/printf", "%s\\n", "--version"]);
    let explained = tapas(&["--explain", "/usr/bin/printf", "%s\\n", "--completions"]);

    assert!(normal.status.success());
    assert_eq!(normal.stdout, b"--help\n");
    assert!(normal.stderr.is_empty());
    assert!(raw.status.success());
    assert_eq!(raw.stdout, b"--version\n");
    assert!(raw.stderr.is_empty());
    assert!(explained.status.success());
    assert_eq!(explained.stdout, b"--completions\n");
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
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git status --short","description":"working tree"}}"#,
        &[("PATH", "/usr/bin:/bin")],
    );

    assert!(eligible.status.success());
    let executable = env!("CARGO_BIN_EXE_tapas");
    let output = String::from_utf8(eligible.stdout).expect("UTF-8 hook output");
    assert!(output.contains("\"permissionDecision\":\"allow\""));
    assert!(output.contains("GIT_OPTIONAL_LOCKS=0"));
    assert!(output.contains("GIT_CONFIG_KEY_1=log.showSignature"));
    assert!(output.contains("GIT_CONFIG_KEY_2=format.pretty"));
    assert!(output.contains(&format!("'{executable}'")));
    assert!(output.contains("'/usr/bin/git' 'status' '--short'"));
    assert!(output.contains("\"description\":\"working tree\""));
    assert!(eligible.stderr.is_empty());

    let diff = tapas_with_stdin(
        &["--hook-eval", "codex"],
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git diff --stat"}}"#,
        &[("PATH", "/usr/bin:/bin")],
    );
    let diff_output = String::from_utf8(diff.stdout).expect("UTF-8 hook output");
    assert!(diff_output.contains("'diff' '--no-ext-diff' '--no-textconv' '--stat'"));

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    let shadow_root =
        std::env::temp_dir().join(format!("tapas-cli-shadow-{}-{unique}", std::process::id()));
    let shadow_bin = shadow_root.join("bin");
    fs::create_dir_all(&shadow_bin).expect("create shadow bin");
    let shadow_git = shadow_bin.join("git");
    fs::write(&shadow_git, b"#!/bin/sh\nexit 0\n").expect("write shadow git");
    fs::set_permissions(&shadow_git, fs::Permissions::from_mode(0o755))
        .expect("make shadow git executable");
    let shadow_input = format!(
        "{{\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\",\"cwd\":{:?},\"tool_input\":{{\"command\":\"git status\"}}}}",
        shadow_root.to_str().expect("UTF-8 shadow path")
    );
    let shadowed = tapas_with_stdin(
        &["--hook-eval", "codex"],
        shadow_input.as_bytes(),
        &[("PATH", shadow_bin.to_str().expect("UTF-8 shadow bin"))],
    );
    assert!(shadowed.status.success());
    assert!(shadowed.stdout.is_empty());
    assert!(shadowed.stderr.is_empty());

    let workspace = shadow_root.join("project");
    let nested = workspace.join("subdirectory");
    let workspace_bin = workspace.join("bin");
    fs::create_dir_all(&nested).expect("create nested workspace directory");
    fs::create_dir_all(&workspace_bin).expect("create workspace bin");
    fs::create_dir(workspace.join(".git")).expect("create workspace marker");
    let workspace_git = workspace_bin.join("git");
    fs::write(&workspace_git, b"#!/bin/sh\nexit 0\n").expect("write workspace git");
    fs::set_permissions(&workspace_git, fs::Permissions::from_mode(0o755))
        .expect("make workspace git executable");
    let nested_input = format!(
        "{{\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\",\"cwd\":{:?},\"tool_input\":{{\"command\":\"git status\"}}}}",
        nested.to_str().expect("UTF-8 nested path")
    );
    let nested_path = format!("{}:/usr/bin:/bin", workspace_bin.display());
    let nested_shadow = tapas_with_stdin(
        &["--hook-eval", "codex"],
        nested_input.as_bytes(),
        &[("PATH", &nested_path)],
    );
    fs::remove_dir_all(&shadow_root).expect("remove shadow root");
    let nested_output = String::from_utf8(nested_shadow.stdout).expect("UTF-8 hook output");
    assert!(nested_shadow.status.success());
    assert!(nested_output.contains("'/usr/bin/git' 'status'"));
    assert!(nested_shadow.stderr.is_empty());

    for input in [
        br#"{"tool_name":"Bash","tool_input":{"command":"git status"}}"#.as_slice(),
        br#"{"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"git status"}}"#,
        br#"{"hook_event_name":"PreToolUse","tool_input":{"command":"git status"}}"#,
        br#"{"hook_event_name":"PreToolUse","tool_name":"Shell","tool_input":{"command":"git status"}}"#,
    ] {
        let ignored = tapas_with_stdin(
            &["--hook-eval", "codex"],
            input,
            &[("PATH", "/usr/bin:/bin")],
        );
        assert!(ignored.status.success());
        assert!(ignored.stdout.is_empty(), "input: {input:?}");
        assert!(ignored.stderr.is_empty(), "input: {input:?}");
    }

    let already_wrapped = format!(
        "{{\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\",\"cwd\":\"/tmp\",\"tool_input\":{{\"command\":\"'{executable}' git status\"}}}}"
    );
    let oversized = format!(
        "{{\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\",\"cwd\":\"/tmp\",\"tool_input\":{{\"command\":\"git status\",\"description\":\"{}\"}}}}",
        "x".repeat(64 * 1024)
    );
    for input in [
        b"invalid JSON".to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git status | cat"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git status\nrm -rf x"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git reset --hard"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"./git status"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"rg --pre 'rm -rf .' needle"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"rg --pre-glob='*.md' needle"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"rg --hostname-bin=touch needle"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"rg --search-zip needle"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"rg -z needle"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"find . -delete"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"find . -exec touch changed {} +"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"find . -fprint listing.txt"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git diff --output=changes.patch"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git diff --ext-diff"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git show --textconv HEAD:file"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git log --show-signature"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git log --format=%G?"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"git log --pretty=verify"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"tree -o listing.txt"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"tree --output=listing.txt"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"npm test"}}"#.to_vec(),
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{"command":"unknown command"}}"#.to_vec(),
        already_wrapped.into_bytes(),
        oversized.into_bytes(),
    ] {
        let ignored = tapas_with_stdin(
            &["--hook-eval", "codex"],
            &input,
            &[("PATH", "/usr/bin:/bin")],
        );
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
fn codex_hook_keeps_the_expanded_catalog_outside_its_read_only_allowlist() {
    for command in [
        "diff",
        "golangci-lint",
        "gt",
        "prisma",
        "rake",
        "rspec",
        "rubocop",
        "wc",
    ] {
        let input = format!(
            r#"{{"hook_event_name":"PreToolUse","tool_name":"Bash","cwd":"/tmp","tool_input":{{"command":"{command} --version"}}}}"#,
        );
        let output = tapas_with_stdin(
            &["--hook-eval", "codex"],
            input.as_bytes(),
            &[("PATH", "/usr/bin:/bin")],
        );
        assert!(output.status.success(), "command: {command}");
        assert!(output.stdout.is_empty(), "command: {command}");
        assert!(output.stderr.is_empty(), "command: {command}");
    }
}

#[test]
fn opencode_hook_returns_only_the_rewritten_command() {
    let eligible = tapas_with_stdin(
        &["--hook-eval", "opencode"],
        br#"{"tool_input":{"command":"git status --short"}}"#,
        &[],
    );
    assert!(eligible.status.success());
    let executable = env!("CARGO_BIN_EXE_tapas");
    assert_eq!(
        eligible.stdout,
        format!("'{executable}' git status --short\n").as_bytes()
    );
    assert!(eligible.stderr.is_empty());

    for input in [
        br#"{"tool_input":{"command":"git status | cat"}}"#.as_slice(),
        br#"{"tool_input":{"command":"unknown command"}}"#,
        b"invalid",
    ] {
        let ignored = tapas_with_stdin(&["--hook-eval", "opencode"], input, &[]);
        assert!(ignored.status.success());
        assert!(ignored.stdout.is_empty());
        assert!(ignored.stderr.is_empty());
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
