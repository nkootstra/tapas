use std::fs;
use std::io::{self, Write};
use std::path::Path;

use super::hooks::{
    contains_conflicting_integration, hook_command, hook_entry, hook_exists, parse_config,
    validate_hook,
};
use super::json::Value;
use super::ownership::{
    HookOwnership, Ownership, content_digest, hook_ownership, hook_ownership_record,
    prepare_record_parent, read_ownership, record_bytes, write_hook_ownership,
};
use super::storage::{
    existing_mode, read_optional, reject_symlink, restore_optional, write_atomic,
    write_unique_backup,
};
use super::transaction::Transaction;
use super::{Action, MAX_CONFIG_BYTES, SetupLocation, Target, lossless};

pub(super) fn configure(
    location: &SetupLocation,
    executable: &Path,
    action: Action,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    match action {
        Action::Setup => {
            let hook_command = hook_command(executable.as_os_str(), location.target);
            setup(location, executable, &hook_command, dry_run, stdout, stderr)
        }
        Action::Unsetup => unsetup(location, dry_run, stdout, stderr),
    }
}

fn setup(
    location: &SetupLocation,
    executable: &Path,
    hook_command: &[u8],
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    let SetupLocation {
        config_path,
        ownership_path,
        target,
    } = location;
    if reject_symlink(config_path, stderr)? || reject_symlink(ownership_path, stderr)? {
        return Ok(1);
    }
    if !validate_hook(executable, *target)? {
        stderr.write_all(b"tapas hook evaluator self-check failed\n")?;
        return Ok(1);
    }
    let existing = read_optional(config_path, MAX_CONFIG_BYTES)?;
    let inline_config = if *target == Target::Codex {
        read_optional(&config_path.with_file_name("config.toml"), MAX_CONFIG_BYTES)?
    } else {
        None
    };
    let project_conflict = if *target == Target::Claude {
        let cwd = std::env::current_dir()?;
        let candidates = [
            cwd.join(".claude/settings.json"),
            cwd.join(".claude/settings.local.json"),
        ];
        let mut conflict = None;
        for path in candidates {
            if read_optional(&path, MAX_CONFIG_BYTES)?
                .as_deref()
                .is_some_and(contains_conflicting_integration)
            {
                conflict = Some(path);
                break;
            }
        }
        conflict
    } else {
        None
    };
    let conflict_name = if existing
        .as_deref()
        .is_some_and(contains_conflicting_integration)
    {
        Some(target.config_name())
    } else if inline_config
        .as_deref()
        .is_some_and(contains_conflicting_integration)
    {
        Some("config.toml")
    } else if project_conflict.is_some() {
        Some("active project settings")
    } else {
        None
    };
    if let Some(conflict_name) = conflict_name {
        writeln!(
            stderr,
            "Conflicting command-wrapper integration detected in {}. Remove it first, then run tapas --setup {} again.",
            conflict_name,
            target.name()
        )?;
        return Ok(1);
    }
    let root = match parse_config(existing.as_deref(), target.config_name(), stderr) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(1),
        Err(error) => return Err(error),
    };
    let ownership = read_ownership(ownership_path)?;
    let owned_value = match ownership {
        Ownership::Modified => {
            stderr.write_all(
                b"tapas setup ownership record was modified; configuration left untouched\n",
            )?;
            return Ok(1);
        }
        Ownership::Valid(entry) => Some(entry),
        Ownership::Missing => None,
    };
    let owned_record = owned_value
        .as_ref()
        .and_then(|value| hook_ownership(value, *target));
    if owned_record.is_none()
        && owned_value.as_ref().is_some_and(
            |value| matches!(value.get(b"kind"), Some(Value::String(kind)) if kind == b"hook"),
        )
    {
        stderr.write_all(
            b"tapas hook ownership target or metadata was modified; configuration left untouched\n",
        )?;
        return Ok(1);
    }
    let expected_entry = hook_entry(hook_command);
    if owned_record
        .as_ref()
        .is_some_and(|record| record.path != *config_path)
    {
        return relocate_hook_setup(
            location,
            owned_record.as_ref().unwrap(),
            &expected_entry,
            existing.as_deref(),
            dry_run,
            stdout,
            stderr,
        );
    }
    let owned_entry = owned_value.as_ref().map(|value| {
        owned_record
            .as_ref()
            .map_or_else(|| value.clone(), |record| record.entry.clone())
    });
    let legacy_before = if owned_record.is_none() {
        match (owned_entry.as_ref(), existing.as_deref()) {
            (Some(owned), Some(input)) => {
                match lossless::remove_hook(input, owned) {
                    Ok(lossless::RemoveResult::Removed(bytes)) => Some(bytes),
                    Ok(lossless::RemoveResult::Missing | lossless::RemoveResult::Duplicate)
                    | Err(()) => {
                        stderr.write_all(b"legacy Tapas hook ownership is ambiguous; configuration left untouched\n")?;
                        return Ok(1);
                    }
                }
            }
            (Some(_), None) => {
                stderr.write_all(
                    b"legacy Tapas hook ownership has no configuration; no files were changed\n",
                )?;
                return Ok(1);
            }
            (None, _) => None,
        }
    } else {
        None
    };

    if owned_entry.is_none()
        && existing.as_deref().is_some_and(|input| {
            lossless::tapas_hook_count(input, *target).unwrap_or(usize::MAX) > 0
        })
    {
        stderr.write_all(
            b"a Tapas-looking hook exists without valid ownership; configuration left untouched\n",
        )?;
        return Ok(1);
    }
    warn_on_replacement(
        *target,
        owned_entry.as_ref(),
        &expected_entry,
        executable,
        stderr,
    )?;
    let base = match owned_entry.as_ref() {
        Some(owned) if owned != &expected_entry => match legacy_before.clone() {
            Some(bytes) => Some(bytes),
            None => match existing.as_deref() {
                Some(input) => match lossless::remove_hook(input, owned) {
                    Ok(lossless::RemoveResult::Removed(bytes)) => Some(bytes),
                    Ok(lossless::RemoveResult::Missing | lossless::RemoveResult::Duplicate)
                    | Err(()) => {
                        stderr.write_all(b"tapas-owned hook entry was modified, removed, or duplicated; configuration left untouched\n")?;
                        return Ok(1);
                    }
                },
                None => {
                    stderr.write_all(b"tapas-owned hook entry was modified or removed; configuration left untouched\n")?;
                    return Ok(1);
                }
            },
        },
        Some(owned) => {
            if !matches!(hook_exists(&root, owned), Ok(true)) {
                stderr.write_all(b"tapas-owned hook entry was modified or removed; configuration left untouched\n")?;
                return Ok(1);
            }
            existing.clone()
        }
        None => existing.clone(),
    };
    let (rendered, already_installed) = match lossless::add_hook(base.as_deref(), &expected_entry) {
        Ok(result) => result,
        Err(()) => {
            writeln!(
                stderr,
                "{}: invalid or ambiguous JSON hook configuration",
                target.config_name()
            )?;
            return Ok(1);
        }
    };
    let changed = existing.as_deref() != Some(rendered.as_slice());
    let mut backup_path = owned_record
        .as_ref()
        .and_then(|record| record.backup_path.clone());
    let mut created_backup = false;
    if !dry_run && owned_record.is_none() {
        backup_path = write_unique_backup(
            config_path,
            legacy_before.as_deref().or(existing.as_deref()),
        )?;
        created_backup = backup_path.is_some();
    }
    if changed {
        if dry_run {
            writeln!(stdout, "[dry-run] would update {}", config_path.display())?;
        } else {
            if let Err(error) =
                write_atomic(config_path, &rendered, existing_mode(config_path, 0o600))
            {
                if created_backup && let Some(path) = &backup_path {
                    let _ = fs::remove_file(path);
                }
                return Err(error);
            }
            writeln!(stdout, "updated {}", config_path.display())?;
        }
    } else if already_installed {
        stdout.write_all(b"already installed\n")?;
    }

    if dry_run {
        stdout.write_all(b"[dry-run] would record tapas hook ownership\n")?;
        return Ok(0);
    }
    if !changed && owned_record.is_some() {
        stdout.write_all(b"ok\n")?;
        return Ok(0);
    }
    if let Err(error) = write_hook_ownership(
        ownership_path,
        *target,
        config_path,
        &expected_entry,
        &rendered,
        owned_record
            .as_ref()
            .map_or(existing.is_some(), |record| record.before_existed),
        backup_path.as_deref(),
    ) {
        if changed {
            restore_optional(config_path, existing.as_deref())?;
        }
        if created_backup && let Some(path) = backup_path {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    stdout.write_all(b"ok\n")?;
    Ok(0)
}

fn relocate_hook_setup(
    location: &SetupLocation,
    owned: &HookOwnership,
    expected_entry: &Value,
    destination_before: Option<&[u8]>,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    if reject_symlink(&owned.path, stderr)? {
        return Ok(1);
    }
    let Some(old_before) = read_optional(&owned.path, MAX_CONFIG_BYTES)? else {
        stderr.write_all(
            b"the recorded Tapas hook location is missing; relocation was not attempted\n",
        )?;
        return Ok(1);
    };
    if destination_before.is_some_and(|input| {
        lossless::tapas_hook_count(input, location.target).unwrap_or(usize::MAX) > 0
    }) {
        stderr.write_all(b"the relocation destination already has a Tapas-looking hook without ownership; no files were changed\n")?;
        return Ok(1);
    }
    let (old_after, remove_old) = if content_digest(&old_before) == owned.after_digest {
        if owned.before_existed {
            let Some(backup) = owned.backup_path.as_deref() else {
                stderr.write_all(b"the recorded Tapas hook is missing its restoration backup; relocation was not attempted\n")?;
                return Ok(1);
            };
            let Some(original) = read_optional(backup, MAX_CONFIG_BYTES)? else {
                stderr.write_all(b"the recorded Tapas restoration backup is missing; relocation was not attempted\n")?;
                return Ok(1);
            };
            (original, false)
        } else {
            (Vec::new(), true)
        }
    } else {
        match lossless::remove_hook(&old_before, &owned.entry) {
            Ok(lossless::RemoveResult::Removed(bytes)) => (bytes, false),
            Ok(lossless::RemoveResult::Missing | lossless::RemoveResult::Duplicate) | Err(()) => {
                stderr.write_all(b"the recorded Tapas hook was modified or duplicated; relocation was not attempted\n")?;
                return Ok(1);
            }
        }
    };
    let (destination_after, _) = match lossless::add_hook(destination_before, expected_entry) {
        Ok(result) => result,
        Err(()) => {
            stderr.write_all(
                b"the relocation destination has an invalid or ambiguous hook configuration\n",
            )?;
            return Ok(1);
        }
    };
    if dry_run {
        writeln!(
            stdout,
            "[dry-run] would remove the Tapas hook from {}",
            owned.path.display()
        )?;
        writeln!(
            stdout,
            "[dry-run] would install the Tapas hook in {}",
            location.config_path.display()
        )?;
        stdout.write_all(b"[dry-run] would update tapas hook ownership\n")?;
        return Ok(0);
    }
    let old_mode = existing_mode(&owned.path, 0o600);
    let destination_mode = existing_mode(&location.config_path, 0o600);
    let destination_backup = write_unique_backup(&location.config_path, destination_before)?;
    let transaction = (|| {
        prepare_record_parent(&location.ownership_path)?;
        let ownership = hook_ownership_record(
            location.target,
            &location.config_path,
            expected_entry,
            &destination_after,
            destination_before.is_some(),
            destination_backup.as_deref(),
        );
        let mut transaction = Transaction::new();
        if remove_old {
            transaction.remove_file(&owned.path)?;
        } else {
            transaction.write(&owned.path, old_after, old_mode)?;
        }
        transaction.write(&location.config_path, destination_after, destination_mode)?;
        transaction.write(&location.ownership_path, record_bytes(&ownership), 0o600)?;
        Ok(transaction)
    })();
    let transaction = match transaction {
        Ok(transaction) => transaction,
        Err(error) => {
            if let Some(path) = destination_backup {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
    };
    if let Err(failure) = transaction.commit() {
        if failure.rollback_failures.is_empty() {
            if let Some(path) = destination_backup {
                let _ = fs::remove_file(path);
            }
            return Err(failure.error);
        }
        return Err(io::Error::new(
            failure.error.kind(),
            format!(
                "hook relocation failed ({}); rollback also failed: {}. Recovery backup was retained.",
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
    writeln!(
        stdout,
        "relocated Tapas hook from {} to {}",
        owned.path.display(),
        location.config_path.display()
    )?;
    stdout.write_all(b"ok\n")?;
    Ok(0)
}

fn warn_on_replacement(
    target: Target,
    owned_entry: Option<&Value>,
    expected_entry: &Value,
    executable: &Path,
    stderr: &mut dyn Write,
) -> io::Result<()> {
    if target == Target::Codex && owned_entry.is_some_and(|owned| owned != expected_entry) {
        let build_kind = if env!("TAPAS_BUILD_LABEL").contains("-dev.") {
            "development"
        } else {
            "stable"
        };
        writeln!(
            stderr,
            "warning: replacing the existing Tapas-owned Codex hook with the {build_kind} build {} ({})",
            env!("TAPAS_BUILD_LABEL"),
            executable.display()
        )?;
    }
    Ok(())
}

fn unsetup(
    location: &SetupLocation,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    unsetup_with_remove(location, dry_run, stdout, stderr, |path| {
        fs::remove_file(path)
    })
}

fn unsetup_with_remove(
    location: &SetupLocation,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    remove_ownership: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<i32> {
    let SetupLocation {
        config_path,
        ownership_path,
        target,
    } = location;
    if reject_symlink(config_path, stderr)? || reject_symlink(ownership_path, stderr)? {
        return Ok(1);
    }
    let owned_value = match read_ownership(ownership_path)? {
        Ownership::Missing => {
            let existing = read_optional(config_path, MAX_CONFIG_BYTES)?;
            if existing.as_deref().is_some_and(|input| {
                lossless::tapas_hook_count(input, *target).unwrap_or(usize::MAX) > 0
            }) {
                stderr.write_all(
                    b"a Tapas-looking hook exists without valid ownership; no hook was removed\n",
                )?;
                return Ok(1);
            }
            stdout.write_all(b"not installed\n")?;
            return Ok(0);
        }
        Ownership::Modified => {
            stderr.write_all(b"tapas hook ownership record was modified; no hook was removed\n")?;
            return Ok(1);
        }
        Ownership::Valid(value) => value,
    };
    let owned_record = hook_ownership(&owned_value, *target);
    if owned_record.is_none()
        && matches!(owned_value.get(b"kind"), Some(Value::String(kind)) if kind == b"hook")
    {
        stderr.write_all(
            b"tapas hook ownership target or metadata was modified; no hook was removed\n",
        )?;
        return Ok(1);
    }
    let owned_entry = owned_record
        .as_ref()
        .map_or_else(|| owned_value.clone(), |record| record.entry.clone());
    let Some(existing) = read_optional(config_path, MAX_CONFIG_BYTES)? else {
        stderr.write_all(b"tapas-owned hook configuration is missing; no hook was removed\n")?;
        return Ok(1);
    };
    let (rendered, remove_config) = if let Some(record) = &owned_record {
        if content_digest(&existing) == record.after_digest {
            if record.before_existed {
                let Some(backup) = record.backup_path.as_deref() else {
                    stderr.write_all(b"tapas ownership is missing its restoration backup; configuration left untouched\n")?;
                    return Ok(1);
                };
                let Some(original) = read_optional(backup, MAX_CONFIG_BYTES)? else {
                    stderr.write_all(
                        b"tapas restoration backup is missing; configuration left untouched\n",
                    )?;
                    return Ok(1);
                };
                (original, false)
            } else {
                (Vec::new(), true)
            }
        } else {
            match lossless::remove_hook(&existing, &owned_entry) {
                Ok(lossless::RemoveResult::Removed(bytes)) => (bytes, false),
                Ok(lossless::RemoveResult::Missing | lossless::RemoveResult::Duplicate)
                | Err(()) => {
                    stderr.write_all(b"tapas-owned hook entry was modified or removed; configuration left untouched; restore the exact owned entry or remove the modified hook and ownership record manually\n")?;
                    return Ok(1);
                }
            }
        }
    } else {
        match lossless::remove_hook(&existing, &owned_entry) {
            Ok(lossless::RemoveResult::Removed(bytes)) => (bytes, false),
            Ok(lossless::RemoveResult::Missing | lossless::RemoveResult::Duplicate) | Err(()) => {
                stderr.write_all(b"tapas-owned hook entry was modified or removed; configuration left untouched; restore the exact owned entry or remove the modified hook and ownership record manually\n")?;
                return Ok(1);
            }
        }
    };
    if dry_run {
        writeln!(stdout, "[dry-run] would update {}", config_path.display())?;
        return Ok(0);
    }

    let original_mode = existing_mode(config_path, 0o600);
    let unsetup_backup = write_unique_backup(config_path, Some(&existing))?;
    let config_result = if remove_config {
        fs::remove_file(config_path)
    } else {
        write_atomic(config_path, &rendered, original_mode)
    };
    if let Err(error) = config_result {
        if let Some(path) = &unsetup_backup {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    if let Err(remove_error) = remove_ownership(ownership_path) {
        if let Err(rollback_error) = write_atomic(config_path, &existing, original_mode) {
            return Err(io::Error::new(
                rollback_error.kind(),
                format!(
                    "failed to remove ownership record ({remove_error}); settings rollback failed: {rollback_error}"
                ),
            ));
        }
        if let Some(path) = &unsetup_backup {
            let _ = fs::remove_file(path);
        }
        return Err(remove_error);
    }
    writeln!(stdout, "updated {}", config_path.display())?;
    stdout.write_all(b"ok\n")?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use std::fs::{self, Permissions};
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::atomic::Ordering;

    use super::{unsetup_with_remove, warn_on_replacement};
    use crate::setup::hooks::hook_entry;
    use crate::setup::ownership::write_ownership;
    use crate::setup::{SetupLocation, TEMP_SEQUENCE, Target};

    #[test]
    fn codex_setup_warns_when_replacing_an_owned_hook() {
        let old = hook_entry(b"/old/tapas --hook-eval codex");
        let expected = hook_entry(b"/new/tapas --hook-eval codex");
        let mut stderr = Vec::new();
        warn_on_replacement(
            Target::Codex,
            Some(&old),
            &expected,
            Path::new("/new/tapas"),
            &mut stderr,
        )
        .unwrap();

        assert!(
            String::from_utf8_lossy(&stderr)
                .contains("replacing the existing Tapas-owned Codex hook")
        );
    }

    #[test]
    fn unsetup_rolls_back_settings_when_ownership_removal_fails() {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let home = std::env::temp_dir().join(format!(
            "tapas-unsetup-rollback-test-{}-{sequence}",
            std::process::id()
        ));
        let config_path = home.join(".claude/settings.json");
        let ownership_path = home.join(".tapas/setup/claude.owned");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let original = concat!(
            "{\n",
            "  \"hooks\": {\"PreToolUse\": [{\"matcher\":\"Bash\",\"hooks\":[{\"type\":\"command\",\"command\":\"tapas-hook\",\"timeout\":10}]}]},\n",
            "  \"ratio\": 0.5\n",
            "}\n"
        )
        .as_bytes();
        fs::write(&config_path, original).unwrap();
        fs::set_permissions(&config_path, Permissions::from_mode(0o640)).unwrap();
        write_ownership(&ownership_path, &hook_entry(b"tapas-hook")).unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let error = unsetup_with_remove(
            &SetupLocation {
                config_path: config_path.clone(),
                ownership_path: ownership_path.clone(),
                target: Target::Claude,
            },
            false,
            &mut stdout,
            &mut stderr,
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected ownership deletion failure",
                ))
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read(&config_path).unwrap(), original);
        assert_eq!(
            fs::metadata(&config_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(ownership_path.exists());
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
        fs::remove_dir_all(home).unwrap();
    }
}
