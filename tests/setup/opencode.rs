#[cfg(target_os = "linux")]
use std::ffi::OsString;
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::Command;

use super::support::{TestHome, assert_opencode_plugin_behavior, tapas_with_env};

#[test]
fn opencode_setup_is_idempotent_and_unsetup_removes_only_tapas() {
    let home = TestHome::new();
    let xdg = home.path("xdg");
    let plugin = xdg.join("opencode/plugins/tapas.js");

    let setup = tapas_with_env(
        &home,
        &["--setup", "opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(setup.status.success(), "{:?}", setup.stderr);
    let content = fs::read(&plugin).unwrap();
    assert!(
        content
            .windows(b"tool.execute.before".len())
            .any(|p| p == b"tool.execute.before")
    );
    assert!(
        content
            .windows(b"Bun.spawnSync".len())
            .any(|p| p == b"Bun.spawnSync")
    );
    assert!(
        content
            .windows(b"--hook-eval".len())
            .any(|p| p == b"--hook-eval")
    );
    assert_opencode_plugin_behavior(&plugin);

    let repeated = tapas_with_env(
        &home,
        &["--setup=opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(repeated.status.success());
    assert_eq!(fs::read(&plugin).unwrap(), content);

    let sibling = xdg.join("opencode/plugins/keep.ts");
    fs::write(&sibling, b"export const keep = true;\n").unwrap();
    let unsetup = tapas_with_env(
        &home,
        &["--unsetup", "opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(unsetup.status.success(), "{:?}", unsetup.stderr);
    assert!(!plugin.exists());
    assert_eq!(fs::read(&sibling).unwrap(), b"export const keep = true;\n");
}

#[test]
#[cfg(target_os = "linux")]
fn opencode_setup_rejects_non_utf8_executable_paths_without_writing_files() {
    let source = Path::new(env!("CARGO_BIN_EXE_tapas"));
    let home = TestHome::new_in(source.parent().unwrap());
    let executable = home.0.join(OsString::from_vec(b"tapas-\xff".to_vec()));
    fs::hard_link(source, &executable).unwrap();
    let xdg = home.path("xdg");

    let output = Command::new(&executable)
        .args(["--setup", "opencode"])
        .env_clear()
        .env("HOME", &home.0)
        .env("XDG_CONFIG_HOME", &xdg)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("UTF-8"));
    assert!(!xdg.join("opencode/plugins/tapas.js").exists());
    assert!(!home.path(".tapas/setup/opencode.owned").exists());
}

#[test]
fn opencode_predecessor_requires_force_and_force_removes_only_recognized_files() {
    let home = TestHome::new();
    let xdg = home.path("xdg");
    let plugins = xdg.join("opencode/plugins");
    fs::create_dir_all(&plugins).unwrap();
    let predecessor = plugins.join("rtk.ts");
    let unrelated = plugins.join("notes.ts");
    fs::write(
        &predecessor,
        b"// rtk rewrite\nexport const RtkOpenCodePlugin = async () => ({ \"tool.execute.before\": async () => {} })\n",
    )
    .unwrap();
    fs::write(&unrelated, b"// rtk is mentioned in documentation only\n").unwrap();

    let blocked = tapas_with_env(
        &home,
        &["--setup", "opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert_eq!(blocked.status.code(), Some(1));
    assert!(predecessor.exists());
    assert!(!plugins.join("tapas.js").exists());

    let forced = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(forced.status.success(), "{:?}", forced.stderr);
    assert!(!predecessor.exists());
    assert_eq!(
        fs::read(&unrelated).unwrap(),
        b"// rtk is mentioned in documentation only\n"
    );
    assert!(plugins.join("tapas.js").exists());
}

#[test]
fn opencode_force_refuses_ambiguous_predecessors_and_preserves_unrelated_directories() {
    for symlinked in [false, true] {
        let home = TestHome::new();
        let xdg = home.path("xdg");
        let plugins = xdg.join("opencode/plugins");
        fs::create_dir_all(&plugins).unwrap();
        let predecessor = plugins.join("rtk.ts");
        let target = home.path("unrecognized-rtk.ts");
        fs::write(&target, b"// user-owned file\n").unwrap();
        if symlinked {
            symlink(&target, &predecessor).unwrap();
        } else {
            fs::write(&predecessor, b"// user-owned file\n").unwrap();
        }
        let config = xdg.join("opencode/opencode.json");
        fs::write(&config, b"{\n  \"theme\": \"dark\"\n}\n").unwrap();

        let output = tapas_with_env(
            &home,
            &["--setup", "opencode", "--force"],
            b"",
            &[("XDG_CONFIG_HOME", xdg.as_path())],
        );

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(fs::read(&config).unwrap(), b"{\n  \"theme\": \"dark\"\n}\n");
        assert!(fs::symlink_metadata(&predecessor).is_ok());
        assert!(!plugins.join("tapas.js").exists());
        assert!(!home.path(".tapas/setup/opencode.owned").exists());
    }

    let home = TestHome::new();
    let xdg = home.path("xdg");
    let unrelated = xdg.join("opencode/plugins/smll-proxy");
    fs::create_dir_all(&unrelated).unwrap();
    let output = tapas_with_env(
        &home,
        &["--setup", "opencode"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(unrelated.is_dir());

    let home = TestHome::new();
    let xdg = home.path("xdg");
    let root = xdg.join("opencode");
    fs::create_dir_all(root.join("plugins")).unwrap();
    let target = home.path("opencode.json");
    fs::write(&target, b"{\"plugin\":[]}\n").unwrap();
    symlink(&target, root.join("opencode.json")).unwrap();
    fs::write(
        root.join("plugins/rtk.ts"),
        b"// rtk rewrite\nexport const RtkOpenCodePlugin = async () => ({ \"tool.execute.before\": async () => {} })\n",
    )
    .unwrap();
    let output = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        fs::symlink_metadata(root.join("opencode.json"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(target).unwrap(), b"{\"plugin\":[]}\n");
    assert!(root.join("plugins/rtk.ts").exists());
    assert!(!root.join("plugins/tapas.js").exists());
}

#[test]
fn opencode_force_removes_smll_registration_losslessly() {
    let home = TestHome::new();
    let xdg = home.path("xdg");
    let root = xdg.join("opencode");
    let proxy = root.join("plugins/smll-proxy");
    fs::create_dir_all(&proxy).unwrap();
    fs::write(
        proxy.join("index.ts"),
        b"// smll-proxy\nexport const SmllProxyPlugin = async () => ({ \"tool.execute.before\": async () => {} });\n",
    )
    .unwrap();
    fs::write(
        proxy.join("package.json"),
        b"{\"name\":\"smll-proxy\",\"main\":\"index.ts\"}\n",
    )
    .unwrap();
    let smll_ownership = home.path(".smll/setup/opencode.owned");
    fs::create_dir_all(smll_ownership.parent().unwrap()).unwrap();
    fs::write(
        &smll_ownership,
        b"smll-setup-v1\nee10f25e7743059b\n265e0e6e87cc138a\n90fcd6f4857a96d7",
    )
    .unwrap();
    let registered = proxy.as_os_str().as_encoded_bytes();
    let mut config =
        b"{\n  \"theme\" : \"dark\",\n  \"plugin\" : [\n    \"keep-me\",\n    \"".to_vec();
    config.extend_from_slice(registered);
    config.extend_from_slice(b"\"\n  ],\n  \"tail\": 1.00e+2\n}\n");
    fs::write(root.join("opencode.json"), &config).unwrap();

    let output = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert!(output.status.success(), "{:?}", output.stderr);
    let cleaned = fs::read(root.join("opencode.json")).unwrap();
    assert!(
        cleaned
            .windows(b"\"keep-me\"".len())
            .any(|part| part == b"\"keep-me\"")
    );
    assert!(
        cleaned
            .windows(b"1.00e+2".len())
            .any(|part| part == b"1.00e+2")
    );
    assert!(
        !cleaned
            .windows(registered.len())
            .any(|part| part == registered)
    );
    assert!(!proxy.exists());
    assert!(!smll_ownership.exists());
    assert!(root.join("plugins/tapas.js").exists());
}

#[test]
fn opencode_force_preserves_jsonc_and_external_predecessor_conflicts() {
    let home = TestHome::new();
    let xdg = home.path("xdg");
    let root = xdg.join("opencode");
    fs::create_dir_all(&root).unwrap();
    let jsonc = root.join("opencode.jsonc");
    let jsonc_before = b"{ // user comments\n  \"plugin\": [\"smll-proxy\"]\n}\n";
    fs::write(&jsonc, jsonc_before).unwrap();

    let blocked = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[("XDG_CONFIG_HOME", xdg.as_path())],
    );
    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(fs::read(&jsonc).unwrap(), jsonc_before);
    assert!(!root.join("plugins/tapas.js").exists());
    assert!(!home.path(".tapas/setup/opencode.owned").exists());

    fs::remove_file(&jsonc).unwrap();
    let custom = home.path("custom-opencode");
    let external = custom.join("plugins/rtk.ts");
    fs::create_dir_all(external.parent().unwrap()).unwrap();
    let external_before = b"// rtk rewrite\nexport const RtkOpenCodePlugin = async () => ({ \"tool.execute.before\": async () => {} })\n";
    fs::write(&external, external_before).unwrap();

    let blocked = tapas_with_env(
        &home,
        &["--setup", "opencode", "--force"],
        b"",
        &[
            ("XDG_CONFIG_HOME", xdg.as_path()),
            ("OPENCODE_CONFIG_DIR", custom.as_path()),
        ],
    );
    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(fs::read(&external).unwrap(), external_before);
    assert!(!root.join("plugins/tapas.js").exists());
    assert!(!home.path(".tapas/setup/opencode.owned").exists());
}
