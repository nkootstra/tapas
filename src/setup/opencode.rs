use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use self::plugin::{
    generate as opencode_plugin, owned_matches as opencode_owned_matches,
    ownership as opencode_ownership, paths_are_utf8 as opencode_paths_are_utf8,
};
use self::predecessor::{
    Predecessor, contains_predecessor_marker, opencode_config_without_predecessors,
    opencode_external_conflicts, opencode_predecessors, smll_digest,
    smll_opencode_ownership_recognized,
};
use super::hooks::validate_hook;
use super::ownership::{Ownership, prepare_record_parent, read_ownership, record_bytes};
use super::storage::{existing_mode, read_optional, reject_symlink, write_unique_backup};
use super::transaction::Transaction;
use super::{Action, MAX_CONFIG_BYTES, SetupLocation, Target};

mod plugin;
mod predecessor;

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
    if !opencode_paths_are_utf8(&location.config_path, executable) {
        stderr.write_all(
            b"OpenCode setup requires UTF-8 executable and configuration paths; no files were changed\n",
        )?;
        return Ok(1);
    }
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
    let smll_directory = plugin_dir.join("smll-proxy");
    let smll_artifact_digests = ["index.ts", "package.json"]
        .map(|name| {
            predecessors
                .iter()
                .find(|item| item.recognized && item.path == smll_directory.join(name))
                .map(|item| smll_digest(&item.content))
        })
        .into_iter()
        .collect::<Option<Vec<_>>>();
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home).join(".smll/setup/opencode.owned");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                predecessors.push(Predecessor {
                    path,
                    recognized: false,
                    content: Vec::new(),
                });
            }
            Ok(_) => {
                let content = read_optional(&path, MAX_CONFIG_BYTES)?.unwrap_or_default();
                predecessors.push(Predecessor {
                    recognized: smll_artifact_digests.as_deref().is_some_and(|digests| {
                        smll_opencode_ownership_recognized(&content, &digests[0], &digests[1])
                    }),
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
    let removes_smll_directory = predecessors
        .iter()
        .any(|item| item.path.parent() == Some(smll_directory.as_path()));
    for item in &predecessors {
        if read_optional(&item.path, MAX_CONFIG_BYTES)?.as_deref() != Some(item.content.as_slice())
        {
            return Err(io::Error::other(format!(
                "predecessor changed during setup: {}",
                item.path.display()
            )));
        }
    }
    let mut transaction = Transaction::new();
    for item in &predecessors {
        transaction.remove_file(&item.path)?;
    }
    if removes_smll_directory {
        transaction.remove_empty_directory(&smll_directory)?;
    }
    if let Some(bytes) = config_after.as_deref().filter(|_| config_changed) {
        transaction.write(&config_path, bytes.to_vec(), config_mode)?;
    }
    transaction.write(&location.config_path, plugin.clone(), original_mode)?;
    transaction.write(&location.ownership_path, record_bytes(&expected), 0o600)?;
    let mut created_backups = Vec::new();
    let prepare = (|| {
        if config_changed
            && let Some(path) = write_unique_backup(&config_path, config_before.as_deref())?
        {
            created_backups.push(path);
        }
        if let Some(path) = write_unique_backup(&location.config_path, current.as_deref())? {
            created_backups.push(path);
        }
        prepare_record_parent(&location.ownership_path)
    })();
    if let Err(error) = prepare {
        for path in created_backups {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    if let Err(failure) = transaction.commit() {
        if failure.rollback_failures.is_empty() {
            for path in created_backups {
                let _ = fs::remove_file(path);
            }
            return Err(failure.error);
        }
        return Err(io::Error::new(
            failure.error.kind(),
            format!(
                "OpenCode setup failed ({}); rollback also failed: {}. Recovery backups were retained.",
                failure.error,
                failure
                    .rollback_failures
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
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
    let mut transaction = Transaction::new();
    transaction.remove_file(&location.config_path)?;
    transaction.remove_file(&location.ownership_path)?;
    if let Err(failure) = transaction.commit() {
        if let Some(rollback) = failure.rollback_failures.first() {
            return Err(io::Error::new(
                failure.error.kind(),
                format!(
                    "failed to remove OpenCode ownership ({}); plugin rollback failed: {}",
                    failure.error, rollback.error
                ),
            ));
        }
        return Err(failure.error);
    }
    writeln!(stdout, "removed {}", location.config_path.display())?;
    stdout.write_all(b"ok\n")?;
    Ok(0)
}
