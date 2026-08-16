#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);
static TAPAS_PROCESS: Mutex<()> = Mutex::new(());

fn temp_dir() -> PathBuf {
    let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/plugin-test-tmp");
    std::fs::create_dir_all(&root).expect("create safe plugin test root");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.join(format!(
        "tapas-plugin-test-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir(&path).expect("create plugin test directory");
    path
}

fn executable(path: &Path, contents: &[u8]) {
    std::fs::write(path, contents).expect("write executable");
    let mut permissions = std::fs::metadata(path)
        .expect("read executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("make executable");
}

fn tapas(home: &Path, args: &[&str]) -> Output {
    let _guard = TAPAS_PROCESS.lock().unwrap();
    Command::new(env!("CARGO_BIN_EXE_tapas"))
        .args(args)
        .env("HOME", home)
        .output()
        .expect("run tapas")
}

fn tapas_in(home: &Path, current_dir: &Path, path: &Path, args: &[&str]) -> Output {
    let _guard = TAPAS_PROCESS.lock().unwrap();
    let search_path = std::env::join_paths(
        std::iter::once(path.to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_tapas"))
        .args(args)
        .env("HOME", home)
        .env("PATH", search_path)
        .current_dir(current_dir)
        .output()
        .expect("run tapas in project")
}

fn hook_in(home: &Path, current_dir: &Path, target: &str, input: &[u8]) -> Output {
    let _guard = TAPAS_PROCESS.lock().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tapas"))
        .args(["--hook-eval", target])
        .env("HOME", home)
        .current_dir(current_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn check_and_test_run_conformance_without_conferring_trust() {
    let directory = temp_dir();
    let home = directory.join("home");
    std::fs::create_dir(&home).unwrap();
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins/node/acme-tools.mjs");

    let check = tapas(
        &home,
        &["--plugin", "check", "--", plugin.to_str().unwrap()],
    );
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let report = String::from_utf8(check.stdout).unwrap();
    assert!(report.contains("conforms to tapas-filter v1"));
    assert!(report.contains("does not establish trust, safety, or semantic quality"));
    assert!(!home.join(".tapas/plugins.json").exists());

    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "trust",
                "acme-tools",
                "--",
                plugin.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    let test = tapas(&home, &["--plugin", "test", "acme-tools"]);
    assert!(
        test.status.success(),
        "{}",
        String::from_utf8_lossy(&test.stderr)
    );
    assert!(
        String::from_utf8(test.stdout)
            .unwrap()
            .contains("does not establish trust, safety, or semantic quality")
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn resolve_json_reports_a_binding_without_starting_the_command_or_plugin() {
    let directory = temp_dir();
    let home = directory.join("home");
    std::fs::create_dir(&home).unwrap();
    let command_marker = directory.join("command-started");
    let plugin_marker = directory.join("plugin-started");
    let command = directory.join("quietcmd");
    let plugin = directory.join("quiet-plugin");
    executable(
        &command,
        format!("#!/bin/sh\ntouch '{}'\n", command_marker.display()).as_bytes(),
    );
    executable(
        &plugin,
        format!("#!/bin/sh\ntouch '{}'\n", plugin_marker.display()).as_bytes(),
    );
    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "trust",
                "quiet",
                "--pin",
                "--",
                plugin.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &["--plugin", "bind", "--user", "quiet", "--", "quietcmd"]
        )
        .status
        .success()
    );

    let output = tapas(
        &home,
        &[
            "--plugin",
            "resolve",
            "--json",
            "--",
            command.to_str().unwrap(),
        ],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = tapas::json::parse(&output.stdout).unwrap();
    assert_eq!(report["version"], 1);
    assert_eq!(report["disposition"], "active");
    assert_eq!(report["plugin"], "quiet");
    assert_eq!(report["scope"], "user");
    assert_eq!(report["trust"]["mode"], "pinned");
    assert_eq!(
        report["matched_prefix"],
        tapas::json::parse(br#"["quietcmd"]"#).unwrap()
    );
    assert!(!command_marker.exists());
    assert!(!plugin_marker.exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn trusted_local_plugin_compacts_a_bound_command_without_changing_its_outcome() {
    let directory = temp_dir();
    let home = directory.join("home");
    std::fs::create_dir(&home).expect("create isolated home");
    let plugin = directory.join("acme_tools.py");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/plugins/python/acme_tools.py"
        ),
        &plugin,
    )
    .expect("copy example plugin");
    let mut permissions = std::fs::metadata(&plugin).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&plugin, permissions).unwrap();

    let command = directory.join("acme");
    executable(
        &command,
        b"#!/bin/sh\ni=1\nwhile [ $i -le 20 ]; do printf 'PASS case-%s details details details\\n' \"$i\"; i=$((i + 1)); done\nprintf 'SUMMARY tests=20 failures=1\\n'\nprintf 'WARN retrying flaky worker with verbose context\\n' >&2\nprintf 'WARN retrying flaky worker with verbose context\\n' >&2\nprintf 'ERROR case-7 expected=ready actual=failed\\n' >&2\nexit 42\n",
    );
    let plugin = plugin.to_str().expect("UTF-8 plugin path");
    let command = command.to_str().expect("UTF-8 command path");
    let verbose = Command::new(command)
        .arg("test")
        .output()
        .expect("run verbose command directly");

    let trust = tapas(&home, &["--plugin", "trust", "acme-tools", "--", plugin]);
    assert!(
        trust.status.success(),
        "{}",
        String::from_utf8_lossy(&trust.stderr)
    );
    let bind = tapas(
        &home,
        &["--plugin", "bind", "--user", "acme-tools", "--", command],
    );
    assert!(
        bind.status.success(),
        "{}",
        String::from_utf8_lossy(&bind.stderr)
    );

    let output = tapas(&home, &[command, "test"]);

    assert_eq!(output.status.code(), Some(42));
    assert_eq!(
        output.stdout,
        b"PASS 20 cases\nSUMMARY tests=20 failures=1\n"
    );
    assert_eq!(
        output.stderr,
        b"WARN retrying flaky worker with verbose context (repeated 2 times)\nERROR case-7 expected=ready actual=failed\n"
    );
    assert!(
        output.stdout.len() + output.stderr.len() < verbose.stdout.len() + verbose.stderr.len()
    );

    let explained = tapas(&home, &["--explain", command, "test"]);
    let explanation = String::from_utf8_lossy(&explained.stderr);
    assert!(explanation.contains("plugin=acme-tools"), "{explanation}");
    assert!(explanation.contains("disposition=active"), "{explanation}");
    assert!(explanation.contains("raw="), "{explanation}");
    assert!(explanation.contains("displayed="), "{explanation}");

    std::fs::remove_dir_all(directory).expect("remove plugin test directory");
}

#[test]
fn mutable_path_trust_records_and_refreshes_the_last_conformed_digest() {
    let directory = temp_dir();
    let home = directory.join("home");
    std::fs::create_dir(&home).expect("create isolated home");
    let plugin = directory.join("acme_tools.py");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/plugins/python/acme_tools.py"
        ),
        &plugin,
    )
    .expect("copy example plugin");
    let mut permissions = std::fs::metadata(&plugin).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&plugin, permissions).unwrap();
    let command = directory.join("acme");
    executable(&command, b"#!/bin/sh\nprintf 'PASS case-1 details details details\\n'\nprintf 'SUMMARY tests=1 failures=0\\n'\n");
    let plugin_path = plugin.to_str().unwrap();
    let command_path = command.to_str().unwrap();

    assert!(
        tapas(
            &home,
            &["--plugin", "trust", "acme-tools", "--", plugin_path]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "bind",
                "--user",
                "acme-tools",
                "--",
                command_path
            ]
        )
        .status
        .success()
    );
    let before =
        tapas::json::parse(&std::fs::read(home.join(".tapas/plugins.json")).unwrap()).unwrap();
    let first_digest = before["plugins"]["acme-tools"]["sha256"]
        .as_str()
        .expect("trust records SHA-256");
    assert_eq!(first_digest.len(), 64);

    std::fs::OpenOptions::new()
        .append(true)
        .open(&plugin)
        .unwrap()
        .write_all(b"\n# conformant update\n")
        .unwrap();
    let output = tapas(&home, &[command_path]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"PASS 1 cases\nSUMMARY tests=1 failures=0\n");
    let after =
        tapas::json::parse(&std::fs::read(home.join(".tapas/plugins.json")).unwrap()).unwrap();
    assert_ne!(after["plugins"]["acme-tools"]["sha256"], first_digest);

    std::fs::remove_dir_all(directory).expect("remove plugin test directory");
}

#[test]
fn pinned_plugin_with_changed_bytes_falls_back_to_raw_output() {
    let directory = temp_dir();
    let home = directory.join("home");
    std::fs::create_dir(&home).unwrap();
    let plugin = directory.join("acme_tools.py");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/plugins/python/acme_tools.py"
        ),
        &plugin,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&plugin).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&plugin, permissions).unwrap();
    let command = directory.join("acme");
    executable(&command, b"#!/bin/sh\nprintf 'PASS case-1 details details details\\n'\nprintf 'SUMMARY tests=1 failures=0\\n'\n");
    let plugin_path = plugin.to_str().unwrap();
    let command_path = command.to_str().unwrap();

    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "trust",
                "acme-tools",
                "--pin",
                "--",
                plugin_path
            ]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "bind",
                "--user",
                "acme-tools",
                "--",
                command_path
            ]
        )
        .status
        .success()
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(&plugin)
        .unwrap()
        .write_all(b"\n# changed after pin\n")
        .unwrap();

    let output = tapas(&home, &[command_path]);
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"PASS case-1 details details details\nSUMMARY tests=1 failures=0\n"
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn project_binding_is_inactive_until_its_exact_config_is_approved() {
    let directory = temp_dir();
    let home = directory.join("home");
    let project = directory.join("project");
    let nested = project.join("nested");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir(&nested).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let plugin = directory.join("acme_tools.py");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/plugins/python/acme_tools.py"
        ),
        &plugin,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&plugin).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&plugin, permissions).unwrap();
    executable(
        &bin.join("acme"),
        b"#!/bin/sh\nprintf 'PASS case-1 details details details\\n'\nprintf 'SUMMARY tests=1 failures=0\\n'\n",
    );
    let plugin_path = plugin.to_str().unwrap();

    assert!(
        tapas(
            &home,
            &["--plugin", "trust", "acme-tools", "--", plugin_path]
        )
        .status
        .success()
    );
    let bind = tapas_in(
        &home,
        &nested,
        &bin,
        &["--plugin", "bind", "--project", "acme-tools", "--", "acme"],
    );
    assert!(
        bind.status.success(),
        "{}",
        String::from_utf8_lossy(&bind.stderr)
    );
    let config = tapas::json::parse(&std::fs::read(project.join(".tapas.json")).unwrap()).unwrap();
    assert_eq!(
        std::fs::metadata(project.join(".tapas.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    assert_eq!(
        config,
        tapas::json::parse(
            br#"{"version":1,"filters":[{"command":["acme"],"plugin":"acme-tools"}]}"#
        )
        .unwrap()
    );

    let inactive = tapas_in(&home, &nested, &bin, &["acme"]);
    assert_eq!(
        inactive.stdout,
        b"PASS case-1 details details details\nSUMMARY tests=1 failures=0\n"
    );
    let approve = tapas_in(&home, &nested, &bin, &["--plugin", "approve-project"]);
    assert!(
        approve.status.success(),
        "{}",
        String::from_utf8_lossy(&approve.stderr)
    );
    let active = tapas_in(&home, &nested, &bin, &["acme"]);
    assert_eq!(
        active.stdout,
        b"PASS 1 cases\nSUMMARY tests=1 failures=0\n",
        "{}",
        String::from_utf8_lossy(&active.stderr)
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(project.join(".tapas.json"))
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    let changed = tapas_in(&home, &nested, &bin, &["acme"]);
    assert_eq!(
        changed.stdout,
        b"PASS case-1 details details details\nSUMMARY tests=1 failures=0\n"
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn project_bind_appends_filters_and_resolve_prefers_the_longest_prefix() {
    let directory = temp_dir();
    let home = directory.join("home");
    let project = directory.join("project");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let plugin = directory.join("plugin");
    executable(&plugin, b"#!/bin/sh\nexit 1\n");
    for id in ["broad", "specific"] {
        assert!(
            tapas(
                &home,
                &[
                    "--plugin",
                    "trust",
                    id,
                    "--pin",
                    "--",
                    plugin.to_str().unwrap()
                ]
            )
            .status
            .success()
        );
    }
    assert!(
        tapas_in(
            &home,
            &project,
            &bin,
            &["--plugin", "bind", "--project", "broad", "--", "acme"]
        )
        .status
        .success()
    );
    assert!(
        tapas_in(
            &home,
            &project,
            &bin,
            &[
                "--plugin",
                "bind",
                "--project",
                "specific",
                "--",
                "acme",
                "test"
            ]
        )
        .status
        .success()
    );
    let config = tapas::json::parse(&std::fs::read(project.join(".tapas.json")).unwrap()).unwrap();
    assert_eq!(config["filters"].as_array().unwrap().len(), 2);
    assert!(
        tapas_in(&home, &project, &bin, &["--plugin", "approve-project"])
            .status
            .success()
    );

    let output = tapas_in(
        &home,
        &project,
        &bin,
        &[
            "--plugin",
            "resolve",
            "--json",
            "--",
            "/usr/local/bin/acme",
            "test",
            "unit",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = tapas::json::parse(&output.stdout).unwrap();
    assert_eq!(report["disposition"], "active");
    assert_eq!(report["plugin"], "specific");
    assert_eq!(
        report["matched_prefix"],
        tapas::json::parse(br#"["acme","test"]"#).unwrap()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn resolve_reports_unapproved_invalid_and_ambiguous_project_configuration() {
    let directory = temp_dir();
    let home = directory.join("home");
    let project = directory.join("project");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let resolve = || {
        tapas_in(
            &home,
            &project,
            &bin,
            &["--plugin", "resolve", "--json", "--", "custom", "run"],
        )
    };

    std::fs::write(
        project.join(".tapas.json"),
        br#"{"version":1,"filters":[{"command":["custom"],"plugin":"one"}]}"#,
    )
    .unwrap();
    assert_eq!(
        tapas::json::parse(&resolve().stdout).unwrap()["disposition"],
        "repo-unapproved"
    );

    std::fs::write(
        project.join(".tapas.json"),
        br#"{"version":1,"settings":{}}"#,
    )
    .unwrap();
    assert_eq!(
        tapas::json::parse(&resolve().stdout).unwrap()["disposition"],
        "invalid-config"
    );

    std::fs::write(
        project.join(".tapas.json"),
        br#"{"version":1,"filters":[{"command":["custom"],"plugin":"one"},{"command":["custom"],"plugin":"two"}]}"#,
    )
    .unwrap();
    assert!(
        tapas_in(&home, &project, &bin, &["--plugin", "approve-project"])
            .status
            .success()
    );
    assert_eq!(
        tapas::json::parse(&resolve().stdout).unwrap()["disposition"],
        "ambiguous"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn agent_hooks_wrap_only_authorized_dynamic_routes_and_codex_stays_static() {
    let directory = temp_dir();
    let home = directory.join("home");
    let project = directory.join("project");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let plugin = directory.join("plugin");
    executable(&plugin, b"#!/bin/sh\nexit 1\n");
    assert!(
        tapas(
            &home,
            &["--plugin", "trust", "agent", "--", plugin.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert!(
        tapas_in(
            &home,
            &project,
            &bin,
            &[
                "--plugin",
                "bind",
                "--project",
                "agent",
                "--",
                "custom",
                "run"
            ]
        )
        .status
        .success()
    );
    let unapproved_event = format!(
        r#"{{"cwd":{:?},"tool_input":{{"command":"custom run"}}}}"#,
        project.to_str().unwrap()
    );
    for target in ["claude", "opencode"] {
        assert!(
            hook_in(&home, &project, target, unapproved_event.as_bytes())
                .stdout
                .is_empty()
        );
    }
    assert!(
        tapas_in(&home, &project, &bin, &["--plugin", "approve-project"])
            .status
            .success()
    );
    let event = format!(
        r#"{{"cwd":{:?},"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"custom run"}}}}"#,
        project.to_str().unwrap()
    );
    for target in ["claude", "opencode"] {
        let output = hook_in(&home, &project, target, event.as_bytes());
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(" custom run"),
            "target={target}, output={:?}",
            output.stdout
        );
    }
    for target in ["claude", "opencode"] {
        assert!(
            hook_in(
                &home,
                &project,
                target,
                br#"{"tool_input":{"command":"custom run"}}"#,
            )
            .stdout
            .is_empty()
        );
    }
    assert!(
        hook_in(&home, &project, "codex", event.as_bytes())
            .stdout
            .is_empty()
    );

    assert!(
        tapas(
            &home,
            &["--plugin", "bind", "--user", "agent", "--", "usercustom"]
        )
        .status
        .success()
    );
    let user_event = format!(
        r#"{{"cwd":{:?},"tool_input":{{"command":"usercustom"}}}}"#,
        project.to_str().unwrap()
    );
    assert!(
        hook_in(&home, &project, "claude", user_event.as_bytes())
            .stdout
            .is_empty()
    );
    assert!(tapas(&home, &["--plugin", "pin", "agent"]).status.success());
    assert!(
        String::from_utf8_lossy(&hook_in(&home, &project, "claude", user_event.as_bytes()).stdout)
            .contains(" usercustom")
    );

    std::fs::write(
        project.join(".tapas.json"),
        br#"{"version":1,"filters":[{"command":["custom","run"],"plugin":"agent"}],"settings":{}}"#,
    )
    .unwrap();
    for target in ["claude", "opencode"] {
        assert!(
            hook_in(&home, &project, target, event.as_bytes())
                .stdout
                .is_empty()
        );
        assert!(
            hook_in(&home, &project, target, user_event.as_bytes())
                .stdout
                .is_empty()
        );
    }
    let operator_event = format!(
        r#"{{"cwd":{:?},"tool_input":{{"command":"custom run | tee out"}}}}"#,
        project.to_str().unwrap()
    );
    assert!(
        hook_in(&home, &project, "claude", operator_event.as_bytes())
            .stdout
            .is_empty()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn selected_plugin_decline_is_terminal_and_preserves_both_streams() {
    let directory = temp_dir();
    let home = directory.join("home");
    std::fs::create_dir(&home).unwrap();
    let plugin = directory.join("decline.py");
    executable(
        &plugin,
        b"#!/usr/bin/env python3\nimport json, sys\nprint(json.dumps({'protocol':'tapas-filter','versions':[1]}), flush=True)\njson.loads(sys.stdin.readline())\nprint(json.dumps({'version':1,'result':'decline'}), flush=True)\n",
    );
    let bin = directory.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let command = bin.join("declinecmd");
    executable(
        &command,
        b"#!/bin/sh\nprintf 'PASS case-1 details details details\\nSUMMARY tests=1 failures=0\\n'\nprintf 'WARN original diagnostic details\\n' >&2\n",
    );
    let plugin = plugin.to_str().unwrap();
    assert!(
        tapas(&home, &["--plugin", "trust", "decliner", "--", plugin])
            .status
            .success()
    );
    assert!(
        tapas(
            &home,
            &["--plugin", "bind", "--user", "decliner", "--", "declinecmd",]
        )
        .status
        .success()
    );

    let output = tapas_in(&home, &directory, &bin, &["declinecmd"]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        b"PASS case-1 details details details\nSUMMARY tests=1 failures=0\n"
    );
    assert_eq!(output.stderr, b"WARN original diagnostic details\n");
    let explained = tapas_in(&home, &directory, &bin, &["--explain", "declinecmd"]);
    assert!(
        String::from_utf8_lossy(&explained.stderr).contains("disposition=fallback"),
        "{}",
        String::from_utf8_lossy(&explained.stderr)
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn plugin_hello_timeout_returns_original_output_promptly() {
    let directory = temp_dir();
    let home = directory.join("home");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let plugin = directory.join("slow.py");
    executable(
        &plugin,
        b"#!/usr/bin/env python3\nimport json, time\ntime.sleep(5)\nprint(json.dumps({'protocol':'tapas-filter','versions':[1]}), flush=True)\n",
    );
    executable(
        &bin.join("slowcmd"),
        b"#!/bin/sh\nprintf 'original stdout\\n'\nprintf 'original stderr\\n' >&2\n",
    );
    assert!(
        tapas(
            &home,
            &["--plugin", "trust", "slow", "--", plugin.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &["--plugin", "bind", "--user", "slow", "--", "slowcmd"]
        )
        .status
        .success()
    );

    let started = std::time::Instant::now();
    let output = tapas_in(&home, &directory, &bin, &["slowcmd"]);

    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    assert_eq!(output.stdout, b"original stdout\n");
    assert_eq!(output.stderr, b"original stderr\n");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn plugin_route_is_snapshotted_before_the_wrapped_command_runs() {
    let directory = temp_dir();
    let home = directory.join("home");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let plugin_body = |output: &str| {
        format!(
            "#!/usr/bin/env python3\nimport json, sys\nprint(json.dumps({{'protocol':'tapas-filter','versions':[1]}}), flush=True)\njson.loads(sys.stdin.readline())\nprint(json.dumps({{'version':1,'result':'transform','evidence':'fact-complete','stdout_b64':'{output}','stderr_b64':''}}), flush=True)\n"
        )
    };
    let first = directory.join("first.py");
    let second = directory.join("second.py");
    executable(&first, plugin_body("QQo=").as_bytes());
    executable(&second, plugin_body("Qgo=").as_bytes());
    executable(
        &bin.join("snapcmd"),
        b"#!/bin/sh\nprintf '{\"version\":1,\"bindings\":[{\"plugin\":\"second\",\"command\":[\"snapcmd\"]}]}' > \"$HOME/.tapas/config.json\"\nprintf 'original output that is longer than either plugin result\\n'\n",
    );
    assert!(
        tapas(
            &home,
            &["--plugin", "trust", "first", "--", first.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "trust",
                "second",
                "--",
                second.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &["--plugin", "bind", "--user", "first", "--", "snapcmd"]
        )
        .status
        .success()
    );

    let output = tapas_in(&home, &directory, &bin, &["snapcmd"]);

    assert_eq!(output.stdout, b"A\n");
    assert!(output.stderr.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pin_rejects_a_digest_that_does_not_match_current_plugin_bytes() {
    let directory = temp_dir();
    let home = directory.join("home");
    std::fs::create_dir(&home).unwrap();
    let plugin = directory.join("plugin");
    executable(&plugin, b"#!/bin/sh\nexit 1\n");
    assert!(
        tapas(
            &home,
            &["--plugin", "trust", "demo", "--", plugin.to_str().unwrap()]
        )
        .status
        .success()
    );

    let output = tapas(
        &home,
        &["--plugin", "pin", "demo", "--sha256", &"0".repeat(64)],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn approve_project_rejects_a_digest_that_does_not_match_current_config_bytes() {
    let directory = temp_dir();
    let home = directory.join("home");
    let project = directory.join("project");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::write(
        project.join(".tapas.json"),
        b"{\"version\":1,\"filters\":[]}",
    )
    .unwrap();

    let output = tapas_in(
        &home,
        &project,
        &bin,
        &["--plugin", "approve-project", "--sha256", &"0".repeat(64)],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match"));
    assert!(!home.join(".tapas/projects.json").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn trust_accepts_combined_flags_and_rejects_duplicates() {
    let directory = temp_dir();
    let home = directory.join("home");
    std::fs::create_dir(&home).unwrap();
    let plugin = directory.join("plugin");
    executable(&plugin, b"#!/bin/sh\nexit 1\n");
    let plugin = plugin.to_str().unwrap();

    assert!(
        tapas(&home, &["--plugin", "trust", "demo", "--", plugin])
            .status
            .success()
    );
    let state =
        tapas::json::parse(&std::fs::read(home.join(".tapas/plugins.json")).unwrap()).unwrap();
    let digest = state["plugins"]["demo"]["sha256"]
        .as_str()
        .expect("trust records SHA-256")
        .to_owned();

    let combined = tapas(
        &home,
        &[
            "--plugin",
            "trust",
            "demo",
            "--pin",
            "--replace",
            "--sha256",
            &digest,
            "--",
            plugin,
        ],
    );
    assert!(
        combined.status.success(),
        "{}",
        String::from_utf8_lossy(&combined.stderr)
    );
    let state =
        tapas::json::parse(&std::fs::read(home.join(".tapas/plugins.json")).unwrap()).unwrap();
    assert_eq!(state["plugins"]["demo"]["pinned"], true);
    assert_eq!(
        state["plugins"]["demo"]["sha256"].as_str(),
        Some(digest.as_str())
    );

    let duplicate = tapas(
        &home,
        &["--plugin", "trust", "demo", "--pin", "--pin", "--", plugin],
    );
    assert!(!duplicate.status.success());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn trust_rejects_a_symbolic_link_to_an_executable_plugin() {
    let directory = temp_dir();
    let home = directory.join("home");
    std::fs::create_dir(&home).unwrap();
    let target = directory.join("plugin-target");
    let link = directory.join("plugin-link");
    executable(&target, b"#!/bin/sh\nexit 1\n");
    symlink(&target, &link).unwrap();

    let output = tapas(
        &home,
        &["--plugin", "trust", "demo", "--", link.to_str().unwrap()],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("symbolic link"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn plugin_process_gets_neutral_cwd_and_does_not_inherit_arbitrary_secrets() {
    let directory = temp_dir();
    let home = directory.join("home");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let plugin = directory.join("environment.py");
    executable(
        &plugin,
        b"#!/usr/bin/env python3\nimport base64,json,os,sys\nprint(json.dumps({'protocol':'tapas-filter','versions':[1]}),flush=True)\nr=json.loads(sys.stdin.readline())\noriginal=base64.b64decode(r['cwd_b64']).decode()\nok='TAPAS_TEST_SECRET' not in os.environ and os.getcwd()!=original\nprint(json.dumps({'version':1,'result':'transform','evidence':'fact-complete','stdout_b64':base64.b64encode(b'OK\\n' if ok else b'BAD\\n').decode(),'stderr_b64':''}),flush=True)\n",
    );
    executable(
        &bin.join("envcmd"),
        b"#!/bin/sh\nprintf 'original long output\\n'\n",
    );
    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "trust",
                "environment",
                "--",
                plugin.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &["--plugin", "bind", "--user", "environment", "--", "envcmd"]
        )
        .status
        .success()
    );

    let search_path = std::env::join_paths(
        std::iter::once(bin.clone())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tapas"))
        .arg("envcmd")
        .env("HOME", &home)
        .env("PATH", search_path)
        .env("TAPAS_TEST_SECRET", "must-not-leak")
        .current_dir(&directory)
        .output()
        .unwrap();

    assert_eq!(output.stdout, b"OK\n");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn plugin_protocol_distinguishes_a_signaled_command_while_cli_keeps_shell_code() {
    let directory = temp_dir();
    let home = directory.join("home");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let plugin = directory.join("signal.py");
    executable(
        &plugin,
        b"#!/usr/bin/env python3\nimport base64,json,sys\nprint(json.dumps({'protocol':'tapas-filter','versions':[1]}),flush=True)\nr=json.loads(sys.stdin.readline())\nok=r['status']=={'kind':'signaled','signal':15}\nprint(json.dumps({'version':1,'result':'transform','evidence':'fact-complete','stdout_b64':base64.b64encode(b'S\\n' if ok else b'BAD\\n').decode(),'stderr_b64':''}),flush=True)\n",
    );
    executable(
        &bin.join("sigcmd"),
        b"#!/bin/sh\nprintf 'long original output\\n'\nkill -TERM $$\n",
    );
    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "trust",
                "signal",
                "--",
                plugin.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &["--plugin", "bind", "--user", "signal", "--", "sigcmd"]
        )
        .status
        .success()
    );

    let output = tapas_in(&home, &directory, &bin, &["sigcmd"]);

    assert_eq!(output.status.code(), Some(143));
    assert_eq!(output.stdout, b"S\n");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn plugin_response_timeout_is_terminal_raw_with_a_short_test_deadline() {
    let directory = temp_dir();
    let home = directory.join("home");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let plugin = directory.join("slow-response.py");
    executable(
        &plugin,
        b"#!/usr/bin/env python3\nimport json,sys,time\nprint(json.dumps({'protocol':'tapas-filter','versions':[1]}),flush=True)\njson.loads(sys.stdin.readline())\ntime.sleep(2)\n",
    );
    executable(
        &bin.join("slowresp"),
        b"#!/bin/sh\nprintf 'original response timeout output\\n'\n",
    );
    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "trust",
                "slow-response",
                "--",
                plugin.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "bind",
                "--user",
                "slow-response",
                "--",
                "slowresp"
            ]
        )
        .status
        .success()
    );
    let search_path = std::env::join_paths(
        std::iter::once(bin.clone())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();

    let started = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_tapas"))
        .arg("slowresp")
        .env("HOME", &home)
        .env("PATH", search_path)
        .env("TAPAS_PLUGIN_TIMEOUT_MS", "50")
        .current_dir(&directory)
        .output()
        .unwrap();

    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    assert_eq!(output.stdout, b"original response timeout output\n");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn protocol_failures_are_terminal_raw_for_crash_malformed_extra_truncated_and_diagnostics() {
    let directory = temp_dir();
    let home = directory.join("home");
    let bin = directory.join("bin");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let marker = directory.join("mode");
    let plugin = directory.join("matrix.py");
    let source = format!(
        "#!/usr/bin/env python3\nimport base64,json,pathlib,sys\nprint(json.dumps({{'protocol':'tapas-filter','versions':[1]}}),flush=True)\nr=json.loads(sys.stdin.readline())\np=pathlib.Path({marker:?})\nif p.exists():\n m=p.read_text()\n if m=='crash': sys.exit(7)\n if m=='malformed': print('{{',flush=True); sys.exit(0)\n if m=='extra': print('{{\"version\":1,\"result\":\"decline\"}}\\n{{}}',flush=True); sys.exit(0)\n if m=='truncated': sys.stdout.write('{{\"version\":1'); sys.stdout.flush(); sys.exit(0)\n if m=='diagnostics': sys.stderr.write('x'*65537); sys.stderr.flush()\n if m=='unknown-field': print(json.dumps({{'version':1,'result':'transform','evidence':'fact-complete','stdout_b64':'T0sK','stderr_b64':'','extra':True}}),flush=True); sys.exit(0)\nout=base64.b64decode(r['stdout_b64']); compact=b'OK\\n'\nprint(json.dumps({{'version':1,'result':'transform','evidence':'fact-complete','stdout_b64':base64.b64encode(compact).decode(),'stderr_b64':''}}),flush=True)\n",
        marker = marker
    );
    executable(&plugin, source.as_bytes());
    executable(
        &bin.join("matrixcmd"),
        b"#!/bin/sh\nprintf 'original protocol failure output\\n'\n",
    );
    assert!(
        tapas(
            &home,
            &[
                "--plugin",
                "trust",
                "matrix",
                "--",
                plugin.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert!(
        tapas(
            &home,
            &["--plugin", "bind", "--user", "matrix", "--", "matrixcmd"]
        )
        .status
        .success()
    );

    for mode in [
        "crash",
        "malformed",
        "extra",
        "truncated",
        "diagnostics",
        "unknown-field",
    ] {
        std::fs::write(&marker, mode).unwrap();
        let output = tapas_in(&home, &directory, &bin, &["matrixcmd"]);
        assert_eq!(
            output.stdout, b"original protocol failure output\n",
            "mode={mode}"
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
