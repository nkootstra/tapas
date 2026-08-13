#![cfg(unix)]

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use tapas::process::invocation::{
    PassthroughReason, StreamDecision, classify, classify_stream, requests_exact_output,
};
use tapas::process::{RunOptions, run};

mod common;

struct FakeCommand {
    directory: PathBuf,
    path: PathBuf,
}

impl FakeCommand {
    fn new(name: &str, script: &[u8]) -> Self {
        let directory = common::unique_temp_dir(&std::env::temp_dir(), "tapas-process-test");
        let path = directory.join(name);
        std::fs::write(&path, script).expect("write fake command");
        let mut permissions = std::fs::metadata(&path)
            .expect("read fake command metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("make fake command executable");
        Self { directory, path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FakeCommand {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).expect("remove fake command directory");
    }
}

fn tapas(args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapas"));
    command
        .args(args)
        .env_remove("TAPAS_LOSSLESS")
        .env_remove("TAPAS_STREAM")
        .env_remove("TAPAS_RAW")
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

fn wait_for_file(path: &Path) -> String {
    let started = Instant::now();
    loop {
        if let Ok(contents) = std::fs::read_to_string(path)
            && !contents.is_empty()
        {
            return contents;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_child(child: &mut Child) -> ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll child status") {
            return status;
        }
        if started.elapsed() >= Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("timed out waiting for child");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn child_arguments_and_stdin_are_forwarded_without_shell_parsing() {
    let literal = tapas(
        &["/usr/bin/printf", "%s\n", "$(not-executed)", "*.rs"],
        b"",
        &[],
    );
    assert!(literal.status.success());
    assert_eq!(literal.stdout, b"$(not-executed)\n*.rs\n");
    assert!(literal.stderr.is_empty());

    let piped = tapas(&["/bin/cat"], b"alpha\0beta\xff\n", &[]);
    assert!(piped.status.success());
    assert_eq!(piped.stdout, b"alpha\0beta\xff\n");
    assert!(piped.stderr.is_empty());
}

#[test]
fn stdout_stderr_and_nonzero_exit_status_remain_separate() {
    let output = tapas(
        &[
            "/bin/sh",
            "-c",
            "printf 'child-out\\n'; printf 'child-err\\n' >&2; exit 42",
        ],
        b"",
        &[],
    );

    assert_eq!(output.status.code(), Some(42));
    assert_eq!(output.stdout, b"child-out\n");
    assert_eq!(output.stderr, b"child-err\n");
}

#[test]
fn child_signal_uses_shell_compatible_exit_status_in_buffered_and_raw_modes() {
    for (prefix, expected_stdout) in [
        (
            &[][..],
            b"(tapas: sh exited 143 with no output)\n".as_slice(),
        ),
        (&["--raw", "--"][..], b"".as_slice()),
    ] {
        let mut args = prefix.to_vec();
        args.extend(["/bin/sh", "-c", "kill -TERM $$"]);
        let output = tapas(&args, b"", &[]);

        assert_eq!(output.status.code(), Some(143));
        assert_eq!(output.stdout, expected_stdout);
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn wrapper_forwards_termination_signals_and_maps_the_child_signal_status() {
    let command = FakeCommand::new(
        "signal-child",
        b"#!/bin/sh\nprintf '%s' \"$$\" > \"$1\"\nexec /bin/sleep 30\n",
    );
    let program = command.path().to_str().expect("UTF-8 fake command path");

    for (name, signal) in [
        ("int", libc::SIGINT),
        ("term", libc::SIGTERM),
        ("hup", libc::SIGHUP),
        ("quit", libc::SIGQUIT),
    ] {
        let pid_path = command.directory.join(format!("{name}.pid"));
        let mut tapas = Command::new(env!("CARGO_BIN_EXE_tapas"))
            .args(["--raw", "--", program])
            .arg(&pid_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn tapas");
        let child_pid: libc::pid_t = wait_for_file(&pid_path).parse().expect("parse child pid");

        // SAFETY: tapas.id() names the live wrapper process synchronized above.
        assert_eq!(unsafe { libc::kill(tapas.id() as libc::pid_t, signal) }, 0);
        let status = wait_for_child(&mut tapas);
        let expected = 128 + signal;
        if status.code() != Some(expected) {
            // The pre-fix wrapper dies without forwarding and leaves this child alive.
            // SAFETY: child_pid was written by that child immediately before exec.
            let _ = unsafe { libc::kill(child_pid, libc::SIGKILL) };
        }
        assert_eq!(status.code(), Some(expected), "signal {name}");
    }
}

#[test]
fn wrapper_forwards_signals_to_the_dedicated_child_process_group() {
    let descendant = FakeCommand::new(
        "signal-descendant",
        b"#!/bin/sh\ntrap 'printf received > \"$2\"; exit 0' TERM\nprintf '%s' \"$$\" > \"$1\"\nwhile :; do sleep 30; done\n",
    );
    let leader = FakeCommand::new(
        "signal-group",
        b"#!/bin/sh\n\"$1\" \"$2\" \"$3\" &\nwhile [ ! -s \"$2\" ]; do sleep 0.01; done\nprintf '%s' \"$$\" > \"$4\"\nwait\n",
    );
    let descendant_pid_path = leader.directory.join("descendant.pid");
    let receipt_path = leader.directory.join("received");
    let leader_pid_path = leader.directory.join("leader.pid");
    let mut tapas = Command::new(env!("CARGO_BIN_EXE_tapas"))
        .arg(leader.path())
        .arg(descendant.path())
        .arg(&descendant_pid_path)
        .arg(&receipt_path)
        .arg(&leader_pid_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tapas");
    let descendant_pid: libc::pid_t = wait_for_file(&descendant_pid_path)
        .parse()
        .expect("parse descendant pid");
    let leader_pid: libc::pid_t = wait_for_file(&leader_pid_path)
        .parse()
        .expect("parse leader pid");

    // SAFETY: both PIDs name live children synchronized by their marker files.
    assert_eq!(unsafe { libc::getpgid(leader_pid) }, leader_pid);
    // SAFETY: descendant_pid names the live descendant synchronized above.
    assert_eq!(unsafe { libc::getpgid(descendant_pid) }, leader_pid);

    let started = Instant::now();
    // SAFETY: tapas.id() names the live wrapper process synchronized above.
    assert_eq!(
        unsafe { libc::kill(tapas.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    let status = wait_for_child(&mut tapas);
    let received = std::fs::read_to_string(&receipt_path).unwrap_or_default();
    if status.code() != Some(128 + libc::SIGTERM) || received != "received" {
        // SAFETY: on failure, stop the synchronized descendant so the test cannot leak it.
        let _ = unsafe { libc::kill(descendant_pid, libc::SIGKILL) };
    }

    assert_eq!(status.code(), Some(128 + libc::SIGTERM));
    assert_eq!(received, "received");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_stopped_child_is_not_treated_as_terminal() {
    let output = tapas(
        &[
            "/bin/sh",
            "-c",
            "(sleep 0.1; kill -CONT $$) & kill -STOP $$; printf 'resumed\\n'",
        ],
        b"",
        &[],
    );

    assert!(output.status.success());
    assert_eq!(output.stdout, b"resumed\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn stdout_and_large_stderr_are_drained_concurrently() {
    let output = tapas(
        &[
            "/bin/sh",
            "-c",
            "i=0; while [ $i -lt 3000 ]; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' >&2; i=$((i + 1)); done; printf 'ok\\n'",
        ],
        b"",
        &[],
    );

    assert!(output.status.success());
    assert_eq!(output.stdout, b"ok\n");
    assert!(output.stderr.len() > 200 * 1024);
}

#[test]
fn descendant_held_pipes_stop_after_the_grace_period() {
    let started = Instant::now();
    let output = tapas(
        &[
            "/bin/sh",
            "-c",
            "(sleep 5) & printf 'direct output\\n'; exit 23",
        ],
        b"",
        &[],
    );
    let elapsed = started.elapsed();

    assert_eq!(output.status.code(), Some(23));
    assert_eq!(output.stdout, b"direct output\n");
    assert!(
        elapsed >= Duration::from_millis(400),
        "elapsed: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(3), "elapsed: {elapsed:?}");
    assert_eq!(
        output.stderr,
        b"(tapas: output incomplete; descendants kept stdout/stderr open after child exit)\n"
    );
}

#[test]
fn capture_overflow_falls_open_byte_exactly_and_preserves_status() {
    let output = tapas(
        &[
            "/bin/sh",
            "-c",
            "dd if=/dev/zero bs=1048576 count=17 2>/dev/null; exit 23",
        ],
        b"",
        &[],
    );

    assert_eq!(output.status.code(), Some(23));
    assert_eq!(output.stdout.len(), 17 * 1024 * 1024);
    assert!(output.stdout.iter().all(|byte| *byte == 0));
    assert!(output.stderr.is_empty());
}

#[test]
fn lossless_and_raw_modes_preserve_bytes_and_do_not_add_empty_hints() {
    let noisy = "same    line\n".repeat(400).into_bytes();
    let lossless = tapas(&["/bin/cat"], &noisy, &[("TAPAS_LOSSLESS", "1")]);
    assert!(lossless.status.success());
    assert_eq!(lossless.stdout, noisy);

    let raw = tapas(&["--raw", "--", "/bin/cat"], &noisy, &[]);
    assert!(raw.status.success());
    assert_eq!(raw.stdout, noisy);

    let empty = tapas(&["--raw", "/bin/sh", "-c", "true"], b"", &[]);
    assert!(empty.status.success());
    assert!(empty.stdout.is_empty());
    assert!(empty.stderr.is_empty());
}

#[test]
fn intentional_passthrough_is_not_reported_as_capture_overflow() {
    let args = ["/usr/bin/printf", "raw\\n"]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let report = run(
        &args,
        &mut stdout,
        &mut stderr,
        RunOptions {
            raw: true,
            explain: false,
        },
    )
    .expect("run raw child");

    assert_eq!(stdout, b"raw\n");
    assert!(stderr.is_empty());
    assert!(!report.capture_overflowed);
    assert!(report.capture_complete);
}

#[test]
fn buffered_empty_commands_emit_exit_aware_tapas_hints() {
    let success = tapas(&["/bin/sh", "-c", "true"], b"", &[]);
    assert!(success.status.success());
    assert_eq!(success.stdout, b"(tapas: sh exited 0 with no output)\n");

    let failure = tapas(&["/bin/sh", "-c", "exit 7"], b"", &[]);
    assert_eq!(failure.status.code(), Some(7));
    assert_eq!(failure.stdout, b"(tapas: sh exited 7 with no output)\n");
}

#[test]
fn transparent_runners_and_streaming_commands_are_classified_conservatively() {
    let args: Vec<OsString> = [
        "uv",
        "run",
        "--project",
        "repo",
        "--offline",
        "--",
        "pytest",
        "-q",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let invocation = classify(&args);
    assert_eq!(invocation.logical_argv, &args[6..]);
    assert_eq!(invocation.passthrough_reason, None);

    let ambiguous: Vec<OsString> = ["npx", "--future", "pytest"]
        .into_iter()
        .map(OsString::from)
        .collect();
    assert_eq!(
        classify(&ambiguous).passthrough_reason,
        Some(PassthroughReason::AmbiguousRunner)
    );

    let bunx: Vec<OsString> = ["bunx", "--no-install", "sample-cli", "usage"]
        .into_iter()
        .map(OsString::from)
        .collect();
    assert_eq!(classify(&bunx).logical_argv, &bunx[2..]);
    assert_eq!(classify(&bunx).passthrough_reason, None);

    let bunx_dev: Vec<OsString> = ["bunx", "--no-install", "sample-cli", "dev"]
        .into_iter()
        .map(OsString::from)
        .collect();
    assert_eq!(classify_stream(&bunx_dev), StreamDecision::Inherit);

    let docker: Vec<OsString> = ["docker", "logs", "--follow", "api"]
        .into_iter()
        .map(OsString::from)
        .collect();
    assert_eq!(classify_stream(&docker), StreamDecision::StreamFilter);

    let start: Vec<OsString> = ["docker", "start", "db"]
        .into_iter()
        .map(OsString::from)
        .collect();
    assert_eq!(classify_stream(&start), StreamDecision::Capture);
}

#[test]
fn transparent_runner_classification_supports_four_layers_and_fails_closed_after_that() {
    let four: Vec<OsString> = ["npx", "uvx", "bunx", "uv", "run", "pytest", "-q"]
        .into_iter()
        .map(OsString::from)
        .collect();
    let classified = classify(&four);
    assert_eq!(classified.logical_argv, &four[5..]);
    assert_eq!(classified.passthrough_reason, None);

    for exact in [
        &["npx", "--future", "vite", "dev"][..],
        &["npx", "uvx", "bunx", "uv", "run", "npx", "pytest"][..],
    ] {
        let exact = exact.iter().map(OsString::from).collect::<Vec<OsString>>();
        let classified = classify(&exact);
        assert_eq!(classified.logical_argv, exact.as_slice());
        assert_eq!(
            classified.passthrough_reason,
            Some(PassthroughReason::AmbiguousRunner)
        );
    }
}

#[test]
fn pnpm_is_only_unwrapped_when_exec_is_the_subcommand() {
    for direct in [
        &["pnpm", "--version"][..],
        &["pnpm", "--filter", "app", "test"][..],
        &["pnpm", "--filter", "exec", "test"][..],
        &["pnpm", "--", "exec", "vite"][..],
        &["pnpm", "--future", "--", "exec", "vite"][..],
    ] {
        let direct = direct.iter().map(OsString::from).collect::<Vec<_>>();
        let classified = classify(&direct);
        assert_eq!(classified.logical_argv, direct.as_slice());
        assert_ne!(
            classified.passthrough_reason,
            Some(PassthroughReason::AmbiguousRunner)
        );
    }

    let exec = ["pnpm", "--filter", "app", "exec", "vite", "dev"]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    assert_eq!(classify(&exec).logical_argv, &exec[4..]);

    for malformed in [
        &["pnpm", "exec", "--future", "vite"][..],
        &["pnpm", "--future", "exec", "vite"][..],
    ] {
        let malformed = malformed.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            classify(&malformed).passthrough_reason,
            Some(PassthroughReason::AmbiguousRunner)
        );
    }
}

#[test]
fn lifecycle_policies_inherit_interactive_and_unbounded_commands() {
    for args in [
        &["vite"][..],
        &["vite", "dev"][..],
        &["vite", "serve"][..],
        &["vite", "preview"][..],
        &["vite", "--host", "0.0.0.0"][..],
        &["vite", "--config", "vite.config.ts", "preview"][..],
        &["vite", "build", "--watch"][..],
        &["vite", "--config", "vite.config.ts", "build", "--watch"][..],
        &["vite", "build", "--watch=true"][..],
        &["esbuild", "app.ts", "--watch"][..],
        &["esbuild", "app.ts", "--watch=forever"][..],
        &["esbuild", "app.ts", "--serve"][..],
        &["esbuild", "app.ts", "--serve=127.0.0.1:8000"][..],
        &["playwright", "test", "--ui"][..],
        &["playwright", "test", "--ui=true"][..],
        &["playwright", "test", "--debug"][..],
        &["playwright", "test", "--headed"][..],
        &["playwright", "show-report"][..],
        &["playwright", "show-trace", "trace.zip"][..],
        &["playwright", "codegen", "example.com"][..],
        &["docker", "run", "alpine"][..],
        &["docker", "compose", "--future", "value", "up"][..],
        &["docker", "compose", "up", "--watch"][..],
        &["docker", "compose", "up", "--menu"][..],
        &["docker", "compose", "up", "--menu=true"][..],
        &["docker", "stats"][..],
        &["docker", "stats", "--no-stream=false"][..],
        &["docker", "stats", "--no-stream", "false"][..],
        &["bat", "--paging=always", "README.md"][..],
        &["batcat", "--paging", "always", "README.md"][..],
        &["ctest", "--repeat=until-fail:3"][..],
        &["ctest", "--repeat", "until-fail:3"][..],
    ] {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(classify_stream(&args), StreamDecision::Inherit, "{args:?}");
    }

    for args in [
        &["vite", "build"][..],
        &["vite", "--config", "vite.config.ts", "build"][..],
        &["esbuild", "app.ts", "--outfile=app.js"][..],
        &["playwright", "test"][..],
        &["docker", "compose", "up", "-d"][..],
        &["docker", "compose", "-f", "compose.yml", "up", "-d"][..],
        &["docker", "compose", "-f", "compose.yml", "ps"][..],
        &["docker", "compose", "up", "--detach"][..],
        &["docker", "compose", "up", "--detach=true"][..],
        &["docker", "compose", "up", "--wait"][..],
        &["docker", "compose", "up", "--no-start"][..],
        &["docker", "stats", "--no-stream"][..],
        &["docker", "stats", "--no-stream=true"][..],
        &["bat", "--paging=auto", "README.md"][..],
        &["ctest", "--output-on-failure"][..],
    ] {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(classify_stream(&args), StreamDecision::Capture, "{args:?}");
    }

    for args in [
        &["docker", "compose", "up"][..],
        &["docker", "compose", "-f", "compose.yml", "up"][..],
        &["docker", "compose", "up", "--detach=false"][..],
        &["docker", "compose", "up", "--detach", "false"][..],
        &["docker", "compose", "up", "--watch=false"][..],
        &["docker", "compose", "up", "--menu=false"][..],
        &["docker-compose", "up"][..],
    ] {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            classify_stream(&args),
            StreamDecision::StreamFilter,
            "{args:?}"
        );
    }

    let nested = ["npx", "vite", "dev"]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    assert_eq!(
        classify_stream(classify(&nested).logical_argv),
        StreamDecision::Inherit
    );
}

#[test]
fn recognized_non_tty_streaming_compacts_by_default_with_a_legacy_opt_out() {
    let command = FakeCommand::new(
        "docker",
        b"#!/bin/sh\nprintf '2026-08-01 10:00:00 INFO ready\\n2026-08-01 10:00:01 INFO ready\\n'\nprintf '2026-08-01 10:00:00 WARN retry\\n2026-08-01 10:00:01 WARN retry\\n' >&2\nexit 42\n",
    );
    let program = command.path().to_str().expect("UTF-8 fake command path");
    let filtered = tapas(&[program, "logs", "-f", "api"], b"", &[]);
    assert_eq!(filtered.status.code(), Some(42));
    assert_eq!(filtered.stdout, "INFO ready ×2\n".as_bytes());
    assert_eq!(filtered.stderr, "WARN retry ×2\n".as_bytes());

    for legacy_value in ["1", "anything"] {
        let legacy = tapas(
            &[program, "logs", "-f", "api"],
            b"",
            &[("TAPAS_STREAM", legacy_value)],
        );
        assert_eq!(legacy.status.code(), Some(42));
        assert_eq!(legacy.stdout, filtered.stdout);
        assert_eq!(legacy.stderr, filtered.stderr);
    }

    let expected_stdout = b"2026-08-01 10:00:00 INFO ready\n2026-08-01 10:00:01 INFO ready\n";
    let expected_stderr = b"2026-08-01 10:00:00 WARN retry\n2026-08-01 10:00:01 WARN retry\n";
    for (prefix, env) in [
        (&["--raw", "--"][..], &[][..]),
        (&[][..], &[("TAPAS_LOSSLESS", "1")][..]),
        (&[][..], &[("TAPAS_STREAM", "0")][..]),
        (&[][..], &[("TAPAS_STREAM", "false")][..]),
    ] {
        let mut args = prefix.to_vec();
        args.extend([program, "logs", "-f", "api"]);
        let raw = tapas(&args, b"", env);
        assert_eq!(raw.status.code(), Some(42));
        assert_eq!(raw.stdout, expected_stdout);
        assert_eq!(raw.stderr, expected_stderr);
    }
}

#[test]
fn compose_up_streams_state_and_deduplicates_prefixed_logs_per_side() {
    let command = FakeCommand::new(
        "docker",
        b"#!/bin/sh\nprintf '[+] Running 1/1\n Container demo-api-1 Started\napi-1 | 2026-08-01 10:00:00 INFO ready\napi-1 | 2026-08-01 10:00:01 INFO ready\n'\nprintf '\\033[31mcustom compose extension\\033[0m  \napi-1 | stderr detail\n' >&2\nexit 23\n",
    );
    let program = command.path().to_str().expect("UTF-8 fake command path");
    let output = tapas(&[program, "compose", "up"], b"", &[]);

    assert_eq!(output.status.code(), Some(23));
    assert_eq!(
        output.stdout,
        "[+] Running 1/1\n Container demo-api-1 Started\napi-1| INFO ready ×2\n".as_bytes()
    );
    assert_eq!(
        output.stderr,
        b"\x1b[31mcustom compose extension\x1b[0m  \napi-1 | stderr detail\n"
    );
}

#[test]
fn exact_output_routes_bypass_live_filters_and_empty_output_hints() {
    let tail = FakeCommand::new(
        "tail",
        b"#!/bin/sh\nprintf '2026-08-01 10:00:00 INFO ready\n2026-08-01 10:00:01 INFO ready\n'\n",
    );
    let tail_program = tail.path().to_str().expect("UTF-8 fake command path");
    let live = tapas(&[tail_program, "-f", "-n", "20", "app.log"], b"", &[]);
    assert!(live.status.success());
    assert_eq!(
        live.stdout,
        b"2026-08-01 10:00:00 INFO ready\n2026-08-01 10:00:01 INFO ready\n"
    );

    let diff = FakeCommand::new("diff", b"#!/bin/sh\nexit 0\n");
    let diff_program = diff.path().to_str().expect("UTF-8 fake command path");
    let empty = tapas(&[diff_program, "--brief", "old", "new"], b"", &[]);
    assert!(empty.status.success());
    assert!(empty.stdout.is_empty());
    assert!(empty.stderr.is_empty());
}

#[test]
fn requested_live_log_metadata_is_preserved() {
    let command = FakeCommand::new(
        "docker",
        b"#!/bin/sh\nprintf '2026-08-01 10:00:00 label=blue INFO ready\n2026-08-01 10:00:01 label=blue INFO ready\n'\n",
    );
    let program = command.path().to_str().expect("UTF-8 fake command path");
    let output = tapas(
        &[
            program,
            "logs",
            "--follow",
            "--timestamps",
            "--details",
            "api",
        ],
        b"",
        &[],
    );

    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"2026-08-01 10:00:00 label=blue INFO ready\n2026-08-01 10:00:01 label=blue INFO ready\n"
    );
}

#[test]
fn jest_watch_unknown_frame_before_recognized_output_fails_open_losslessly() {
    let jest = FakeCommand::new(
        "jest",
        b"#!/bin/sh\nprintf 'custom watch status\\n\\033[2J\\033[HTest Suites: 1 passed, 1 total\\nTests: 1 passed, 1 total\\n'\n",
    );
    let program = jest.path().to_str().expect("UTF-8 fake command path");
    let output = tapas(&[program, "--watch"], b"", &[]);

    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"custom watch status\n\x1b[2J\x1b[HTest Suites: 1 passed, 1 total\nTests: 1 passed, 1 total\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn vitest_watch_unknown_frame_after_recognized_output_only_opens_its_stream_side() {
    let vitest = FakeCommand::new(
        "vitest",
        b"#!/bin/sh\nprintf 'Test Suites: 1 passed, 1 total\\nTests: 1 passed, 1 total\\n\\033[2J\\033[Hcustom watch status\\n\\033[2J\\033[HTest Suites: 1 passed, 1 total\\nTests: 1 passed, 1 total\\n'\nprintf 'Test Suites: 1 passed, 1 total\\nTests: 1 passed, 1 total\\n' >&2\n",
    );
    let program = vitest.path().to_str().expect("UTF-8 fake command path");
    let output = tapas(&[program, "--watch"], b"", &[]);

    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"all tests passed\n\x1b[2J\x1b[Hcustom watch status\n\x1b[2J\x1b[HTest Suites: 1 passed, 1 total\nTests: 1 passed, 1 total\n"
    );
    assert_eq!(output.stderr, b"all tests passed\n");
}

#[test]
fn tsc_and_gh_live_workflows_use_the_default_process_stream_path() {
    let tsc = FakeCommand::new(
        "tsc",
        b"#!/bin/sh\nprintf 'src/app.ts:1:7 - error TS2322: bad type\n        ~\nFound 1 error. Watching for file changes.\n'\n",
    );
    let tsc_program = tsc.path().to_str().expect("UTF-8 fake command path");
    let tsc_output = tapas(&[tsc_program, "--watch"], b"", &[]);
    assert!(tsc_output.status.success());
    assert_eq!(
        tsc_output.stdout,
        b"src/app.ts:1:7 TS2322: bad type\n        ~\nFound 1 error. Watching for file changes.\n"
    );
    assert!(tsc_output.stderr.is_empty());

    let gh = FakeCommand::new(
        "gh",
        b"#!/bin/sh\nprintf 'JOBS\n* build (ID 123)\n  * checkout\n\nJOBS\n\\342\\234\\223 build in 2s (ID 123)\n\n'\n",
    );
    let gh_program = gh.path().to_str().expect("UTF-8 fake command path");
    let gh_output = tapas(&[gh_program, "run", "watch"], b"", &[]);
    assert!(gh_output.status.success());
    assert_eq!(
        gh_output.stdout,
        b"build: running\nbuild: running->passed\n"
    );
    assert!(gh_output.stderr.is_empty());
}

#[test]
fn streaming_preserves_signals_and_descendant_pipe_diagnostics() {
    let signaled = FakeCommand::new("docker", b"#!/bin/sh\nkill -TERM $$\n");
    let program = signaled.path().to_str().expect("UTF-8 fake command path");
    let output = tapas(&[program, "logs", "-f", "api"], b"", &[]);
    assert_eq!(output.status.code(), Some(143));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());

    let retained = FakeCommand::new(
        "docker",
        b"#!/bin/sh\n(sleep 5) &\nprintf '2026-08-01 10:00:00 ready\\n'\n",
    );
    let program = retained.path().to_str().expect("UTF-8 fake command path");
    let started = Instant::now();
    let output = tapas(&[program, "logs", "-f", "api"], b"", &[]);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(output.stdout, b"ready\n");
    assert_eq!(
        output.stderr,
        b"(tapas: output incomplete; descendants kept stdout/stderr open after child exit)\n"
    );
}

#[test]
fn query_machine_and_exact_output_contracts_bypass_filtering() {
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    for values in [
        &["docker", "ps", "--help"][..],
        &["ruff", "rule", "E501"][..],
        &["tsc", "--showConfig"][..],
    ] {
        assert_eq!(
            classify(&args(values)).passthrough_reason,
            Some(PassthroughReason::Query),
            "{values:?}"
        );
    }
    assert_eq!(
        classify(&args(&["pytest", "--", "--version"])).passthrough_reason,
        None
    );
    assert_eq!(classify(&args(&["ps", "-h"])).passthrough_reason, None);

    for values in [
        &["rg", "--json", "needle"][..],
        &["kubectl", "get", "pods", "-ojson"][..],
        &["docker", "container", "inspect", "api"][..],
        &["aws", "sts", "get-caller-identity", "--query=x"][..],
        &["jq", "."][..],
        &["ps", "-opid,comm"][..],
        &["psql", "-At"][..],
        &["systemctl", "show", "nginx"][..],
    ] {
        assert_eq!(
            classify(&args(values)).passthrough_reason,
            Some(PassthroughReason::MachineOutput),
            "{values:?}"
        );
    }

    for values in [
        &["find", ".", "-print0"][..],
        &["ls", "-l"][..],
        &["tree", "--filelimit", "20"][..],
        &["git", "log", "--format=%H"][..],
    ] {
        assert!(requests_exact_output(&args(values)), "{values:?}");
    }
    for values in [
        &["find", ".", "-type", "f"][..],
        &["ls", "-a"][..],
        &["tree", "-L", "2"][..],
        &["git", "log"][..],
    ] {
        assert!(!requests_exact_output(&args(values)), "{values:?}");
    }
}

#[test]
fn invocation_policy_is_byte_safe_and_stops_option_scans_at_the_terminator() {
    let invalid = OsString::from_vec(vec![b'-', b'-', 0xff]);

    assert_eq!(
        classify(&[
            OsString::from("docker"),
            invalid.clone(),
            OsString::from("--help"),
        ])
        .passthrough_reason,
        Some(PassthroughReason::Query),
    );
    assert_eq!(
        classify(&[
            OsString::from("docker"),
            OsString::from("--"),
            OsString::from("--format=json"),
        ])
        .passthrough_reason,
        None,
    );
    assert!(!requests_exact_output(&[
        OsString::from("find"),
        OsString::from("--"),
        OsString::from("-print0"),
    ]));
    assert!(!requests_exact_output(&[
        OsString::from("git"),
        OsString::from("log"),
        OsString::from("--"),
        OsString::from("--format=%H"),
    ]));
}

#[test]
fn expanded_output_shaping_policies_remain_byte_exact() {
    fn exact(values: &[&str]) -> bool {
        requests_exact_output(&values.iter().map(OsString::from).collect::<Vec<_>>())
    }

    for values in [
        &["cargo", "test", "--message-format=json"] as &[&str],
        &["cargo", "metadata", "--json"],
        &[
            "cargo",
            "nextest",
            "run",
            "--message-format",
            "libtest-json",
        ],
        &["nextest", "run", "--message-format=libtest-json-plus"],
        &["go", "test", "-json", "./..."],
        &["jest", "--json"],
        &["jest", "--reporters", "jest-junit"],
        &["vitest", "--reporter=json", "--outputFile=results.json"],
        &["playwright", "test", "--reporter", "junit"],
        &["prisma", "migrate", "diff", "--script"],
        &[
            "prisma",
            "--schema",
            "db.prisma",
            "migrate",
            "diff",
            "--output=diff.sql",
        ],
        &["rspec", "--format", "json"],
        &["rubocop", "--format=json", "--out", "offenses.json"],
        &["golangci-lint", "run", "--out-format", "json"],
        &["dotnet", "test", "--logger", "trx"],
        &["gt", "log", "--format=json"],
        &["diff", "--unified=3", "old", "new"],
        &["head", "--bytes=20", "file"],
        &["tail", "-n20", "file"],
        &["psql", "--output", "rows.txt"],
        &["psql", "--command", "\\copy records to stdout with csv"],
        &["curl", "-vv", "https://example.test"],
        &["curl", "--trace-ascii", "trace.log", "https://example.test"],
        &["curl", "--write-out=%{json}", "https://example.test"],
    ] {
        assert!(exact(values), "{values:?}");
    }

    for values in [
        &["playwright", "test", "--reporter=line"] as &[&str],
        &["vitest", "--reporter=dot"],
        &["cargo", "test", "--", "--message-format=json"],
        &["curl", "https://example.test", "--", "--trace", "trace.log"],
    ] {
        assert!(!exact(values), "{values:?}");
    }
}

#[test]
fn expanded_interactive_lifecycle_policies_inherit_the_terminal() {
    fn stream(values: &[&str]) -> StreamDecision {
        classify_stream(&values.iter().map(OsString::from).collect::<Vec<_>>())
    }

    for values in [
        &["prisma", "migrate", "dev"] as &[&str],
        &["prisma", "migrate", "reset"],
        &["prisma", "--schema", "db.prisma", "migrate", "dev"],
        &["rspec", "--bisect"],
        &["rubocop", "--lsp"],
        &["rubocop", "--mcp"],
        &["rubocop", "--server"],
        &["psql"],
        &["psql", "postgresql://localhost/app"],
        &["psql", "-U", "app", "-h", "localhost", "app"],
        &["psql", "--", "-c"],
        &["nextest", "run", "--debugger"],
        &["cargo", "nextest", "run", "--no-capture"],
        &["cargo", "nextest", "run", "--stress-count=infinite"],
        &["gt", "stack", "--interactive"],
        &["graphite", "--interactive", "submit"],
    ] {
        assert_eq!(stream(values), StreamDecision::Inherit, "{values:?}");
    }

    for values in [
        &["prisma", "migrate", "deploy"] as &[&str],
        &["rspec", "--", "--bisect"],
        &["rubocop", "--", "--lsp"],
        &["psql", "--command", "select 1"],
        &["psql", "-Atcselect 1"],
        &["psql", "-Al"],
        &["psql", "--file=report.sql"],
        &["psql", "--help"],
        &["nextest", "run", "--stress-count=10"],
        &["cargo", "nextest", "run", "--", "--no-capture"],
        &["gt", "stack", "--interactive=false"],
    ] {
        assert_eq!(stream(values), StreamDecision::Capture, "{values:?}");
    }
}

#[test]
fn explain_reports_the_process_outcome_without_state_claims() {
    let output = tapas(
        &["--explain", "/bin/sh", "-c", "printf 'visible\\n'"],
        b"",
        &[],
    );

    assert!(output.status.success());
    assert_eq!(output.stdout, b"visible\n");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 explain output");
    assert!(stderr.contains("(tapas explain: filter=passthrough"));
    assert!(stderr.contains("exit=0 history=not-recorded)"));
}

#[test]
fn git_wrapper_dispatch_compacts_success_and_preserves_failed_streams() {
    let git = FakeCommand::new(
        "git",
        b"#!/bin/sh\n\
          if [ \"$1\" = checkout ]; then\n\
            printf \"Switched to branch 'feature-x'\\n\" >&2\n\
            exit 0\n\
          fi\n\
          printf 'failed stdout\\n'\n\
          printf 'failed stderr\\n' >&2\n\
          exit 7\n",
    );

    let success_args = [
        git.path().as_os_str().to_owned(),
        OsString::from("checkout"),
    ];
    let mut success_stdout = Vec::new();
    let mut success_stderr = Vec::new();
    let success = run(
        &success_args,
        &mut success_stdout,
        &mut success_stderr,
        RunOptions::default(),
    )
    .expect("run successful Git command");
    assert_eq!(success.exit_code, 0);
    assert_eq!(success.filter_name, "git");
    assert!(success_stdout.is_empty());
    assert_eq!(success_stderr, b"^ feature-x\n");

    let failure_args = [git.path().as_os_str().to_owned(), OsString::from("status")];
    let mut failure_stdout = Vec::new();
    let mut failure_stderr = Vec::new();
    let failure = run(
        &failure_args,
        &mut failure_stdout,
        &mut failure_stderr,
        RunOptions::default(),
    )
    .expect("run failed Git command");
    assert_eq!(failure.exit_code, 7);
    assert_eq!(failure.filter_name, "passthrough");
    assert_eq!(failure_stdout, b"failed stdout\n");
    assert_eq!(failure_stderr, b"failed stderr\n");
}

#[test]
fn successful_unrecognized_git_output_remains_a_byte_exact_passthrough() {
    let git = FakeCommand::new(
        "git",
        b"#!/bin/sh\ni=0\nwhile [ \"$i\" -lt 600 ]; do\n  printf 'custom stdout\\n'\n  i=$((i + 1))\ndone\nprintf 'custom stderr\\n' >&2\n",
    );
    let args = [git.path().as_os_str().to_owned(), OsString::from("config")];
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let report = run(&args, &mut stdout, &mut stderr, RunOptions::default())
        .expect("run Git command without a compactor");

    assert_eq!(report.filter_name, "passthrough");
    assert_eq!(report.evidence, tapas::filters::EvidenceClass::ByteExact);
    assert_eq!(stdout, b"custom stdout\n".repeat(600));
    assert_eq!(stderr, b"custom stderr\n");
}

#[test]
fn bare_git_output_remains_composable_with_content_filters() {
    let git = FakeCommand::new(
        "git",
        b"#!/bin/sh\ni=0\nwhile [ \"$i\" -lt 600 ]; do\n  printf 'bare output\\n'\n  i=$((i + 1))\ndone\n",
    );
    let args = [git.path().as_os_str().to_owned()];
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let report =
        run(&args, &mut stdout, &mut stderr, RunOptions::default()).expect("run bare Git command");

    assert_eq!(report.filter_name, "generic");
    assert_eq!(report.evidence, tapas::filters::EvidenceClass::FactComplete);
    assert_eq!(stdout, "bare output ×600\n".as_bytes());
    assert!(stderr.is_empty());
}

#[test]
fn git_pull_and_push_compaction_keeps_descriptors_separate() {
    let git = FakeCommand::new(
        "git",
        b"#!/bin/sh\n\
          if [ \"$1\" = pull ]; then\n\
            printf 'Updating 43fe7da..2cee6f5\\nFast-forward\\n d.txt | 1 +\\n 1 file changed, 1 insertion(+)\\n'\n\
            printf 'From /fixture\\n   43fe7da..2cee6f5  main -> origin/main\\n' >&2\n\
            exit 0\n\
          fi\n\
          printf \"branch 'main' set up to track 'origin/main'.\\n\"\n\
          printf 'To /fixture\\n * [new branch]      main -> main\\n' >&2\n",
    );

    for (subcommand, expected_stdout, expected_stderr) in [
        (
            "pull",
            b"@ fast-forward 43fe7da..2cee6f5\n+1/-0 files=1\n".as_slice(),
            b"< 43fe7da..2cee6f5 main -> origin/main\n".as_slice(),
        ),
        ("push", b"".as_slice(), b"+ new main -> main\n".as_slice()),
    ] {
        let args = [
            git.path().as_os_str().to_owned(),
            OsString::from(subcommand),
        ];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let report = run(&args, &mut stdout, &mut stderr, RunOptions::default())
            .expect("run successful Git transfer");

        assert_eq!(report.exit_code, 0);
        assert_eq!(report.filter_name, "git");
        assert_eq!(stdout, expected_stdout);
        assert_eq!(stderr, expected_stderr);
    }
}

#[test]
fn recognized_filter_output_on_both_descriptors_fails_open() {
    let cargo = FakeCommand::new(
        "cargo",
        b"#!/bin/sh\n\
          printf '   Compiling demo v0.1.0\\n'\n\
          printf '    Finished dev [unoptimized] target(s) in 0.1s\\n'\n\
          printf 'warning: retained diagnostic\\n' >&2\n",
    );
    let args = [cargo.path().as_os_str().to_owned(), OsString::from("build")];
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let report = run(&args, &mut stdout, &mut stderr, RunOptions::default())
        .expect("run build with output on both descriptors");

    assert_eq!(report.filter_name, "passthrough");
    assert_eq!(
        stdout,
        b"   Compiling demo v0.1.0\n    Finished dev [unoptimized] target(s) in 0.1s\n"
    );
    assert_eq!(stderr, b"warning: retained diagnostic\n");
}

#[test]
fn fact_complete_test_failures_compact_without_changing_the_exit_status() {
    let cargo = FakeCommand::new(
        "cargo",
        b"#!/bin/sh\n\
          printf 'running 1 test\\n'\n\
          printf 'test tests::boom ... FAILED\\n'\n\
          printf '%s\\n' \"---- tests::boom stdout ----\"\n\
          printf \"thread 'tests::boom' panicked at src/lib.rs:12:3:\\n\"\n\
          printf 'assertion failed: expected useful fact\\n'\n\
          printf 'test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; finished in 0.01s\\n'\n\
          exit 101\n",
    );
    let args = [cargo.path().as_os_str().to_owned(), OsString::from("test")];
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let report = run(&args, &mut stdout, &mut stderr, RunOptions::default())
        .expect("run failed Cargo test command");

    assert_eq!(report.exit_code, 101);
    assert_eq!(report.filter_name, "test-tools");
    assert!(
        stdout
            .windows(b"tests::boom".len())
            .any(|part| part == b"tests::boom")
    );
    assert!(
        stdout
            .windows(b"src/lib.rs:12:3".len())
            .any(|part| part == b"src/lib.rs:12:3")
    );
    assert!(stdout.len() < report.input_bytes);
    assert!(stderr.is_empty());
}

#[test]
fn filtered_human_ls_uses_the_stable_c_locale() {
    let ls = FakeCommand::new(
        "ls",
        b"#!/bin/sh\nprintf '%s/%s\\n' \"${LC_ALL-unset}\" \"${LANG-unset}\"\n",
    );
    let args = [ls.path().as_os_str().to_owned()];
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let report =
        run(&args, &mut stdout, &mut stderr, RunOptions::default()).expect("run human ls command");

    assert_eq!(report.exit_code, 0);
    assert_eq!(stdout, b"C/C\n");
    assert!(stderr.is_empty());
}
