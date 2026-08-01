#![cfg(unix)]

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tapas::process::invocation::{
    PassthroughReason, StreamDecision, classify, classify_stream, requests_exact_output,
};
use tapas::process::{RunOptions, run};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct FakeCommand {
    directory: PathBuf,
    path: PathBuf,
}

impl FakeCommand {
    fn new(name: &str, script: &[u8]) -> Self {
        let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "tapas-process-test-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).expect("create fake command directory");
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
    assert_eq!(success_stdout, b"^ feature-x\n");
    assert!(success_stderr.is_empty());

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
