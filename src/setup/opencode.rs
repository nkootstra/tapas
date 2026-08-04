use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::filters::contains_ignore_ascii_case;

use super::hooks::validate_hook;
use super::json::Value;
use super::ownership::{Ownership, read_ownership, write_ownership};
use super::storage::{
    existing_mode, read_optional, reject_symlink, restore_optional, write_atomic,
    write_unique_backup,
};
use super::{Action, MAX_CONFIG_BYTES, SetupLocation, Target, json, lossless};

pub(super) fn configure_opencode(
    location: &SetupLocation,
    executable: &Path,
    action: Action,
    dry_run: bool,
    force: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    if reject_symlink(&location.config_path, stderr)?
        || reject_symlink(&location.ownership_path, stderr)?
    {
        return Ok(1);
    }
    match action {
        Action::Setup => setup_opencode(location, executable, dry_run, force, stdout, stderr),
        Action::Unsetup => unsetup_opencode(location, dry_run, stdout, stderr),
    }
}

fn setup_opencode(
    location: &SetupLocation,
    executable: &Path,
    dry_run: bool,
    force: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    if !validate_hook(executable, Target::OpenCode)? {
        stderr.write_all(b"tapas hook evaluator self-check failed\n")?;
        return Ok(1);
    }
    let plugin = opencode_plugin(executable);
    let expected = opencode_ownership(&location.config_path, executable, &plugin);
    let current = read_optional(&location.config_path, MAX_CONFIG_BYTES)?;
    let ownership = read_ownership(&location.ownership_path)?;
    match (&ownership, current.as_deref()) {
        (Ownership::Modified, _) => {
            stderr.write_all(
                b"tapas setup ownership record was modified; configuration left untouched\n",
            )?;
            return Ok(1);
        }
        (Ownership::Missing, Some(_)) => {
            stderr.write_all(b"an unowned tapas.js already exists; ownership cannot be proven, so no files were changed\n")?;
            return Ok(1);
        }
        (Ownership::Valid(owned), Some(bytes))
            if !opencode_owned_matches(owned, &location.config_path, bytes) =>
        {
            stderr.write_all(
                b"tapas-owned OpenCode plugin was modified or relocated; no files were changed\n",
            )?;
            return Ok(1);
        }
        (Ownership::Valid(_), None) => {
            stderr.write_all(b"tapas-owned OpenCode plugin is missing; no files were changed\n")?;
            return Ok(1);
        }
        _ => {}
    }

    let plugin_dir = location.config_path.parent().expect("plugin path parent");
    let mut predecessors = opencode_predecessors(plugin_dir)?;
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home).join(".smll/setup/opencode.owned");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                predecessors.push(Predecessor {
                    path,
                    recognized: false,
                    content: Vec::new(),
                    mode: 0,
                });
            }
            Ok(_) => {
                let content = read_optional(&path, MAX_CONFIG_BYTES)?.unwrap_or_default();
                predecessors.push(Predecessor {
                    recognized: contains_ignore_ascii_case(&content, b"opencode"),
                    mode: existing_mode(&path, 0o600),
                    path,
                    content,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let config_path = plugin_dir
        .parent()
        .expect("OpenCode config root")
        .join("opencode.json");
    if reject_symlink(&config_path, stderr)? {
        return Ok(1);
    }
    let config_before = read_optional(&config_path, MAX_CONFIG_BYTES)?;
    let config_after = match config_before.as_deref() {
        Some(input) => match opencode_config_without_predecessors(input, plugin_dir) {
            Ok(bytes) => Some(bytes),
            Err(()) => {
                stderr.write_all(
                    b"opencode.json is invalid, JSONC, or ambiguous; no files were changed\n",
                )?;
                return Ok(1);
            }
        },
        None => None,
    };
    let config_changed = config_after.as_deref() != config_before.as_deref();
    let jsonc_path = config_path.with_extension("jsonc");
    if read_optional(&jsonc_path, MAX_CONFIG_BYTES)?
        .as_deref()
        .is_some_and(contains_predecessor_marker)
    {
        writeln!(
            stderr,
            "predecessor registration detected in {}; JSONC is read-only and must be cleaned manually",
            jsonc_path.display()
        )?;
        return Ok(1);
    }
    let external_conflicts = opencode_external_conflicts(plugin_dir)?;
    if !external_conflicts.is_empty() {
        writeln!(
            stderr,
            "OpenCode predecessor integration detected outside the managed user plugin directory: {}. Remove it manually before installing Tapas.",
            external_conflicts[0].display()
        )?;
        return Ok(1);
    }
    if predecessors.iter().any(|item| !item.recognized) {
        writeln!(
            stderr,
            "an ambiguous predecessor file exists at {}; no files were changed",
            predecessors
                .iter()
                .find(|item| !item.recognized)
                .unwrap()
                .path
                .display()
        )?;
        return Ok(1);
    }
    if (!predecessors.is_empty() || config_changed) && !force {
        writeln!(
            stderr,
            "OpenCode predecessor integration detected at {}. Re-run with --force to remove the recognized OpenCode integration and install Tapas.",
            predecessors
                .first()
                .map_or(config_path.as_path(), |item| item.path.as_path())
                .display()
        )?;
        return Ok(1);
    }

    let changed =
        current.as_deref() != Some(plugin.as_slice()) || !predecessors.is_empty() || config_changed;
    if dry_run {
        for predecessor in &predecessors {
            writeln!(
                stderr,
                "warning: [dry-run] would remove recognized predecessor {}",
                predecessor.path.display()
            )?;
        }
        if config_changed {
            writeln!(
                stderr,
                "warning: [dry-run] would remove recognized predecessor registrations from {}",
                config_path.display()
            )?;
        }
        if changed {
            writeln!(
                stdout,
                "[dry-run] would install {}",
                location.config_path.display()
            )?;
        } else {
            stdout.write_all(b"already installed\n")?;
        }
        stdout.write_all(b"[dry-run] would record tapas OpenCode ownership\n")?;
        return Ok(0);
    }
    if !changed && matches!(ownership, Ownership::Valid(_)) {
        stdout.write_all(b"already installed\nok\n")?;
        return Ok(0);
    }

    let original_mode = existing_mode(&location.config_path, 0o600);
    let config_mode = existing_mode(&config_path, 0o600);
    let ownership_before = read_optional(&location.ownership_path, MAX_CONFIG_BYTES)?;
    let smll_directory = plugin_dir.join("smll-proxy");
    let removes_smll_directory = predecessors
        .iter()
        .any(|item| item.path.parent() == Some(smll_directory.as_path()));
    let mut created_backups = Vec::new();
    let mut removed_predecessors = 0_usize;
    let mut config_touched = false;
    let mut plugin_touched = false;
    let mut ownership_touched = false;
    let result = (|| {
        for item in &predecessors {
            if read_optional(&item.path, MAX_CONFIG_BYTES)?.as_deref()
                != Some(item.content.as_slice())
            {
                return Err(io::Error::other(format!(
                    "predecessor changed during setup: {}",
                    item.path.display()
                )));
            }
            fs::remove_file(&item.path)?;
            removed_predecessors += 1;
        }
        if removes_smll_directory {
            match fs::remove_dir(&smll_directory) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(bytes) = config_after.as_deref().filter(|_| config_changed) {
            if let Some(path) = write_unique_backup(&config_path, config_before.as_deref())? {
                created_backups.push(path);
            }
            config_touched = true;
            write_atomic(&config_path, bytes, config_mode)?;
        }
        if let Some(path) = write_unique_backup(&location.config_path, current.as_deref())? {
            created_backups.push(path);
        }
        plugin_touched = true;
        write_atomic(&location.config_path, &plugin, original_mode)?;
        ownership_touched = true;
        write_ownership(&location.ownership_path, &expected)
    })();
    if let Err(error) = result {
        let mut rollback_failures = Vec::new();
        if ownership_touched
            && let Err(rollback) =
                restore_optional(&location.ownership_path, ownership_before.as_deref())
        {
            rollback_failures.push(format!("{}: {rollback}", location.ownership_path.display()));
        }
        if plugin_touched
            && let Err(rollback) = restore_optional(&location.config_path, current.as_deref())
        {
            rollback_failures.push(format!("{}: {rollback}", location.config_path.display()));
        }
        for item in predecessors.iter().take(removed_predecessors) {
            if let Err(rollback) = write_atomic(&item.path, &item.content, item.mode) {
                rollback_failures.push(format!("{}: {rollback}", item.path.display()));
            }
        }
        if config_touched
            && let Err(rollback) = restore_optional(&config_path, config_before.as_deref())
        {
            rollback_failures.push(format!("{}: {rollback}", config_path.display()));
        }
        if rollback_failures.is_empty() {
            for path in created_backups {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        return Err(io::Error::new(
            error.kind(),
            format!(
                "OpenCode setup failed ({error}); rollback also failed: {}. Recovery backups were retained.",
                rollback_failures.join("; ")
            ),
        ));
    }
    for item in &predecessors {
        writeln!(
            stderr,
            "warning: removed recognized predecessor {}",
            item.path.display()
        )?;
    }
    if config_changed {
        writeln!(
            stderr,
            "warning: removed recognized predecessor registrations from {}",
            config_path.display()
        )?;
    }
    if changed {
        writeln!(stdout, "installed {}", location.config_path.display())?;
    } else {
        stdout.write_all(b"already installed\n")?;
    }
    stdout.write_all(b"ok\n")?;
    Ok(0)
}

fn unsetup_opencode(
    location: &SetupLocation,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    let owned = match read_ownership(&location.ownership_path)? {
        Ownership::Missing => {
            if location.config_path.exists() {
                stderr.write_all(b"an unowned tapas.js exists; no file was removed\n")?;
                return Ok(1);
            }
            stdout.write_all(b"not installed\n")?;
            return Ok(0);
        }
        Ownership::Modified => {
            stderr
                .write_all(b"tapas setup ownership record was modified; no file was removed\n")?;
            return Ok(1);
        }
        Ownership::Valid(value) => value,
    };
    let Some(current) = read_optional(&location.config_path, MAX_CONFIG_BYTES)? else {
        stderr.write_all(b"tapas-owned OpenCode plugin is missing; no file was removed\n")?;
        return Ok(1);
    };
    if !opencode_owned_matches(&owned, &location.config_path, &current) {
        stderr.write_all(
            b"tapas-owned OpenCode plugin was modified or relocated; no file was removed\n",
        )?;
        return Ok(1);
    }
    if dry_run {
        writeln!(
            stdout,
            "[dry-run] would remove {}",
            location.config_path.display()
        )?;
        return Ok(0);
    }
    let plugin_mode = existing_mode(&location.config_path, 0o600);
    fs::remove_file(&location.config_path)?;
    if let Err(error) = fs::remove_file(&location.ownership_path) {
        if let Err(rollback) = write_atomic(&location.config_path, &current, plugin_mode) {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "failed to remove OpenCode ownership ({error}); plugin rollback failed: {rollback}"
                ),
            ));
        }
        return Err(error);
    }
    writeln!(stdout, "removed {}", location.config_path.display())?;
    stdout.write_all(b"ok\n")?;
    Ok(0)
}

struct Predecessor {
    path: PathBuf,
    recognized: bool,
    content: Vec<u8>,
    mode: u32,
}

fn opencode_predecessors(plugin_dir: &Path) -> io::Result<Vec<Predecessor>> {
    let candidates = [
        (plugin_dir.join("rtk.ts"), PredecessorKind::RtkPlugin),
        (
            plugin_dir.join("smll-proxy.ts"),
            PredecessorKind::SmllPlugin,
        ),
        (
            plugin_dir.join("smll-proxy.js"),
            PredecessorKind::SmllPlugin,
        ),
        (
            plugin_dir.join("smll-proxy/index.ts"),
            PredecessorKind::SmllPlugin,
        ),
        (
            plugin_dir.join("smll-proxy/package.json"),
            PredecessorKind::SmllPackage,
        ),
    ];
    let mut found = Vec::new();
    for (path, kind) in candidates {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                found.push(Predecessor {
                    path,
                    recognized: false,
                    content: Vec::new(),
                    mode: 0,
                });
            }
            Ok(_) => {
                let content = read_optional(&path, MAX_CONFIG_BYTES)?.unwrap_or_default();
                found.push(Predecessor {
                    recognized: predecessor_content_recognized(&content, kind),
                    mode: existing_mode(&path, 0o600),
                    path,
                    content,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(found)
}

#[derive(Clone, Copy)]
enum PredecessorKind {
    RtkPlugin,
    SmllPlugin,
    SmllPackage,
}

fn predecessor_content_recognized(content: &[u8], kind: PredecessorKind) -> bool {
    match kind {
        PredecessorKind::RtkPlugin => {
            contains_ignore_ascii_case(content, b"tool.execute.before")
                && contains_ignore_ascii_case(content, b"rtk")
                && (contains_ignore_ascii_case(content, b"RtkOpenCodePlugin")
                    || contains_ignore_ascii_case(content, b"rtk rewrite"))
        }
        PredecessorKind::SmllPlugin => {
            contains_ignore_ascii_case(content, b"tool.execute.before")
                && contains_ignore_ascii_case(content, b"smll")
                && (contains_ignore_ascii_case(content, b"SmllProxyPlugin")
                    || contains_ignore_ascii_case(content, b"smll-proxy"))
        }
        PredecessorKind::SmllPackage => {
            contains_ignore_ascii_case(content, b"\"name\"")
                && contains_ignore_ascii_case(content, b"smll-proxy")
                && contains_ignore_ascii_case(content, b"\"main\"")
                && contains_ignore_ascii_case(content, b"index.ts")
        }
    }
}

fn opencode_config_without_predecessors(input: &[u8], plugin_dir: &Path) -> Result<Vec<u8>, ()> {
    let smll_directory = plugin_dir.join("smll-proxy");
    let values = vec![
        smll_directory.as_os_str().as_encoded_bytes().to_vec(),
        plugin_dir
            .join("smll-proxy.ts")
            .as_os_str()
            .as_encoded_bytes()
            .to_vec(),
        plugin_dir
            .join("smll-proxy.js")
            .as_os_str()
            .as_encoded_bytes()
            .to_vec(),
        plugin_dir
            .join("rtk.ts")
            .as_os_str()
            .as_encoded_bytes()
            .to_vec(),
    ];
    lossless::remove_root_array_strings(input, b"plugin", &values).map(|(bytes, _)| bytes)
}

fn contains_predecessor_marker(input: &[u8]) -> bool {
    [b"smll-proxy".as_slice(), b"rtk.ts", b"run-toolkit"]
        .iter()
        .any(|marker| contains_ignore_ascii_case(input, marker))
}

fn opencode_external_conflicts(plugin_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join(".opencode/plugins"));
        roots.push(cwd.join("opencode/plugins"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".opencode/plugins"));
    }
    if let Some(custom) = std::env::var_os("OPENCODE_CONFIG_DIR").filter(|value| !value.is_empty())
    {
        roots.push(PathBuf::from(custom).join("plugins"));
    }
    let managed = fs::canonicalize(plugin_dir).unwrap_or_else(|_| plugin_dir.to_path_buf());
    let mut conflicts = Vec::new();
    for root in roots {
        if fs::canonicalize(&root).unwrap_or_else(|_| root.clone()) == managed {
            continue;
        }
        conflicts.extend(
            opencode_predecessors(&root)?
                .into_iter()
                .map(|item| item.path),
        );
    }
    Ok(conflicts)
}

fn opencode_plugin(executable: &Path) -> Vec<u8> {
    let mut quoted = Vec::new();
    json::write_string(executable.as_os_str().as_encoded_bytes(), &mut quoted);
    let mut output =
        b"// Managed by Tapas. Do not edit; use `tapas --unsetup opencode`.\nconst tapas = "
            .to_vec();
    output.extend_from_slice(&quoted);
    output.extend_from_slice(
        br#";
let warned = false;

export const Tapas = async () => ({
  "tool.execute.before": async (input, output) => {
    if (input.tool !== "bash" || typeof output.args?.command !== "string") return;
    try {
      const result = Bun.spawnSync([tapas, "--hook-eval", "opencode"], {
        stdin: JSON.stringify({ tool_input: { command: output.args.command } }),
        stdout: "pipe",
        stderr: "ignore",
        timeout: 1000,
        maxBuffer: 65536,
      });
      const text = result.exitCode === 0 ? result.stdout.toString() : "";
      const command = text.endsWith("\n") ? text.slice(0, -1) : "";
      if (command) output.args.command = command;
    } catch (error) {
      if (!warned) {
        warned = true;
        console.warn("Tapas OpenCode hook failed; command left unchanged");
      }
    }
  },
});
"#,
    );
    output
}

fn opencode_ownership(path: &Path, executable: &Path, content: &[u8]) -> Value {
    Value::Object(vec![
        (b"kind".to_vec(), Value::String(b"opencode-plugin".to_vec())),
        (
            b"path".to_vec(),
            Value::String(path.as_os_str().as_encoded_bytes().to_vec()),
        ),
        (
            b"executable".to_vec(),
            Value::String(executable.as_os_str().as_encoded_bytes().to_vec()),
        ),
        (b"content".to_vec(), Value::String(content.to_vec())),
    ])
}

fn opencode_owned_matches(owned: &Value, path: &Path, content: &[u8]) -> bool {
    matches!(owned.get(b"kind"), Some(Value::String(kind)) if kind == b"opencode-plugin")
        && matches!(owned.get(b"path"), Some(Value::String(value)) if value == path.as_os_str().as_encoded_bytes())
        && matches!(owned.get(b"content"), Some(Value::String(value)) if value == content)
}
