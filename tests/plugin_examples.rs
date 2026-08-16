#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/plugin-example-test-tmp");
    std::fs::create_dir_all(&root).expect("create safe plugin example test root");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.join(format!(
        "tapas-plugin-example-test-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir(&path).unwrap();
    path
}

fn tapas(home: &Path, path: &Path, args: &[&str]) -> Output {
    let search_path = std::env::join_paths(
        std::iter::once(path.to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_tapas"))
        .args(args)
        .env("HOME", home)
        .env("PATH", search_path)
        .output()
        .unwrap()
}

fn exercise_example(runtime: &str, plugin: &Path, action: &str) {
    if Command::new(runtime).arg("--version").output().is_err() {
        if std::env::var_os("TAPAS_REQUIRE_PLUGIN_EXAMPLES").is_some() {
            panic!("required plugin example runtime {runtime} is unavailable");
        }
        eprintln!("skipping {runtime} plugin example: runtime unavailable");
        return;
    }

    let directory = temp_dir();
    let home = directory.join("home");
    let bin = directory.join("bin");
    let plugins = directory.join("plugins");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&plugins).unwrap();
    let plugin_copy = plugins.join(plugin.file_name().unwrap());
    std::fs::copy(plugin, &plugin_copy).unwrap();
    let plugin = plugin_copy;
    let mut plugin_permissions = std::fs::metadata(&plugin).unwrap().permissions();
    plugin_permissions.set_mode(0o755);
    std::fs::set_permissions(&plugin, plugin_permissions).unwrap();
    let command = bin.join("acme");
    std::fs::write(
        &command,
        format!(
            "#!/bin/sh\ncat '{}'\ncat '{}' >&2\nexit 1\n",
            fixture(action, "stdout.input").display(),
            fixture(action, "stderr.input").display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&command).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&command, permissions).unwrap();

    assert!(
        tapas(
            &home,
            &bin,
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
    assert!(
        tapas(
            &home,
            &bin,
            &["--plugin", "bind", "--user", "acme-tools", "--", "acme"]
        )
        .status
        .success()
    );
    let output = tapas(&home, &bin, &["acme", action]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stdout,
        std::fs::read(fixture(action, "stdout.expected")).unwrap(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stderr,
        std::fs::read(fixture(action, "stderr.expected")).unwrap()
    );
    let input_len = std::fs::metadata(fixture(action, "stdout.input"))
        .unwrap()
        .len()
        + std::fs::metadata(fixture(action, "stderr.input"))
            .unwrap()
            .len();
    assert!(((output.stdout.len() + output.stderr.len()) as u64) < input_len);
    assert!(String::from_utf8_lossy(&output.stdout).contains("widget_spec"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("deprecated flag"));
    std::fs::remove_dir_all(directory).unwrap();
}

fn fixture(action: &str, suffix: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/plugins/fixtures")
        .join(format!("acme-{action}.{suffix}"))
}

#[test]
fn node_example_compacts_multiple_bound_command_shapes() {
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins/node/acme-tools.mjs");
    exercise_example("node", &plugin, "test");
    exercise_example("node", &plugin, "build");
}

#[test]
fn python_example_compacts_checked_in_fixture() {
    let plugin =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins/python/acme_tools.py");
    exercise_example("python3", &plugin, "test");
}
