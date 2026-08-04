mod json;

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicU64;
#[cfg(test)]
use std::sync::atomic::Ordering;

use json::Value;

const MAX_HOOK_INPUT: u64 = 64 * 1024;
const MAX_CONFIG_BYTES: u64 = 8 * 1024 * 1024;
const OWNERSHIP_HEADER: &[u8] = b"tapas-setup-v3\n";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Action {
    Setup,
    Unsetup,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Target {
    Claude,
    Codex,
    OpenCode,
}

struct SetupLocation {
    config_path: std::path::PathBuf,
    ownership_path: std::path::PathBuf,
    target: Target,
}

impl Target {
    pub fn parse(value: &OsStr) -> Option<Self> {
        Self::parse_bytes(value.as_encoded_bytes())
    }

    pub(crate) fn parse_bytes(value: &[u8]) -> Option<Self> {
        match value {
            b"claude" => Some(Self::Claude),
            b"codex" => Some(Self::Codex),
            b"opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }

    fn config_path(self, home: &Path, codex_home: Option<&Path>) -> std::path::PathBuf {
        match self {
            Self::Claude => home.join(".claude").join(self.config_name()),
            Self::Codex => codex_home
                .map_or_else(|| home.join(".codex"), Path::to_path_buf)
                .join(self.config_name()),
            Self::OpenCode => unreachable!("OpenCode uses its plugin path"),
        }
    }

    fn config_name(self) -> &'static str {
        match self {
            Self::Claude => "settings.json",
            Self::Codex => "hooks.json",
            Self::OpenCode => "tapas.js",
        }
    }

    fn grants_rewrite_permission(self) -> bool {
        self == Self::Codex
    }

    fn accepts_command(self, command: &[u8]) -> bool {
        eligible(command) && (self != Self::Codex || codex_read_only(command))
    }

    fn accepts_hook_event(self, value: &Value) -> bool {
        self != Self::Codex
            || matches!(
                (value.get(b"hook_event_name"), value.get(b"tool_name")),
                (Some(Value::String(event)), Some(Value::String(tool)))
                    if event == b"PreToolUse" && tool == b"Bash"
            )
    }
}

pub fn hook_eval(
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    self_check: bool,
) -> io::Result<i32> {
    hook_eval_for_target(Target::Claude, stdin, stdout, stderr, self_check)
}

pub fn hook_eval_for_target(
    target: Target,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    _stderr: &mut dyn Write,
    self_check: bool,
) -> io::Result<i32> {
    if self_check {
        return Ok(i32::from(!target.accepts_command(b"git status")));
    }
    let mut input = Vec::new();
    stdin.take(MAX_HOOK_INPUT + 1).read_to_end(&mut input)?;
    if input.len() as u64 > MAX_HOOK_INPUT {
        return Ok(0);
    }
    let Ok(value) = json::parse(&input) else {
        return Ok(0);
    };
    if !target.accepts_hook_event(&value) {
        return Ok(0);
    }
    let Some(command) = event_command(&value) else {
        return Ok(0);
    };
    let (environment, command) = match target {
        Target::OpenCode if target.accepts_command(command) => {
            let executable = std::env::current_exe()?;
            let mut updated = shell_escape(executable.as_os_str());
            updated.push(b' ');
            updated.extend_from_slice(command);
            stdout.write_all(&updated)?;
            stdout.write_all(b"\n")?;
            return Ok(0);
        }
        Target::Claude if target.accepts_command(command) => (b"".as_slice(), command.to_vec()),
        Target::Codex => {
            let Some(Value::String(cwd)) = value.get(b"cwd") else {
                return Ok(0);
            };
            let Some(command) = codex_command(command, cwd) else {
                return Ok(0);
            };
            (command.environment, command.command)
        }
        Target::Claude | Target::OpenCode => return Ok(0),
    };

    let mut updated_input = value
        .get(b"tool_input")
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "tool_input disappeared"))?;
    let executable = std::env::current_exe()?;
    let mut updated_command = environment.to_vec();
    updated_command.extend_from_slice(&shell_escape(executable.as_os_str()));
    updated_command.push(b' ');
    updated_command.extend_from_slice(&command);
    updated_input
        .insert(b"command", Value::String(updated_command))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "tool_input is not an object"))?;

    let mut hook_output = Vec::with_capacity(2 + usize::from(target.grants_rewrite_permission()));
    hook_output.push((
        b"hookEventName".to_vec(),
        Value::String(b"PreToolUse".to_vec()),
    ));
    if target.grants_rewrite_permission() {
        hook_output.push((
            b"permissionDecision".to_vec(),
            Value::String(b"allow".to_vec()),
        ));
    }
    hook_output.push((b"updatedInput".to_vec(), updated_input));
    let output = Value::Object(vec![(
        b"hookSpecificOutput".to_vec(),
        Value::Object(hook_output),
    )]);
    stdout.write_all(&json::serialize(&output))?;
    stdout.write_all(b"\n")?;
    Ok(0)
}

pub fn configure(
    action: Action,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    configure_for_target(action, Target::Claude, dry_run, stdout, stderr)
}

pub fn configure_for_target(
    action: Action,
    target: Target,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    configure_for_target_with_force(action, target, dry_run, false, stdout, stderr)
}

pub fn configure_for_target_with_force(
    action: Action,
    target: Target,
    dry_run: bool,
    force: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    if force && (action != Action::Setup || target != Target::OpenCode) {
        stderr.write_all(b"tapas agent setup: --force is supported only for OpenCode setup\n")?;
        return Ok(2);
    }
    let Some(home) = std::env::var_os("HOME") else {
        stderr.write_all(b"tapas agent setup: HOME is not set\n")?;
        return Ok(1);
    };
    let executable = std::env::current_exe()?;
    let codex_home = if target == Target::Codex {
        std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
    } else {
        None
    };
    let home = Path::new(&home);
    let ownership_path = home.join(format!(".tapas/setup/{}.owned", target.name()));
    let resolved_path = if target == Target::OpenCode {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("opencode/plugins/tapas.js")
    } else {
        target.config_path(home, codex_home.as_deref())
    };
    let config_path = if action == Action::Unsetup {
        match read_ownership(&ownership_path)? {
            Ownership::Valid(value) => recorded_path(&value).unwrap_or(resolved_path),
            Ownership::Missing | Ownership::Modified => resolved_path,
        }
    } else {
        resolved_path
    };
    let location = SetupLocation {
        config_path,
        ownership_path,
        target,
    };
    configure_at(
        &location,
        &executable,
        action,
        dry_run,
        force,
        stdout,
        stderr,
    )
}

fn configure_at(
    location: &SetupLocation,
    executable: &Path,
    action: Action,
    dry_run: bool,
    force: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    if location.target == Target::OpenCode {
        return configure_opencode(location, executable, action, dry_run, force, stdout, stderr);
    }
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
    let ownership_before = read_optional(&location.ownership_path, MAX_CONFIG_BYTES)?;
    let result = (|| {
        if remove_old {
            fs::remove_file(&owned.path)?;
        } else {
            write_atomic(&owned.path, &old_after, old_mode)?;
        }
        write_atomic(&location.config_path, &destination_after, destination_mode)?;
        write_hook_ownership(
            &location.ownership_path,
            location.target,
            &location.config_path,
            expected_entry,
            &destination_after,
            destination_before.is_some(),
            destination_backup.as_deref(),
        )
    })();
    if let Err(error) = result {
        let mut rollback_failures = Vec::new();
        if let Err(rollback) =
            restore_optional(&location.ownership_path, ownership_before.as_deref())
        {
            rollback_failures.push(format!("{}: {rollback}", location.ownership_path.display()));
        }
        if let Err(rollback) = write_atomic(&owned.path, &old_before, old_mode) {
            rollback_failures.push(format!("{}: {rollback}", owned.path.display()));
        }
        if let Err(rollback) = restore_optional(&location.config_path, destination_before) {
            rollback_failures.push(format!("{}: {rollback}", location.config_path.display()));
        }
        if rollback_failures.is_empty() {
            if let Some(path) = destination_backup {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        return Err(io::Error::new(
            error.kind(),
            format!(
                "hook relocation failed ({error}); rollback also failed: {}. Recovery backup was retained.",
                rollback_failures.join("; ")
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

mod hooks;
mod lossless;
mod opencode;
mod ownership;
mod storage;

use hooks::{
    codex_command, codex_read_only, contains_conflicting_integration, eligible, event_command,
    hook_command, hook_entry, hook_exists, parse_config, shell_escape, validate_hook,
};
#[cfg(test)]
use hooks::{ensure_hook, nested_hook_exists, remove_hook};
use opencode::configure_opencode;
#[cfg(test)]
use ownership::write_ownership;
use ownership::{
    HookOwnership, Ownership, content_digest, hook_ownership, read_ownership, recorded_path,
    write_hook_ownership,
};
use storage::{
    existing_mode, read_optional, reject_symlink, restore_optional, write_atomic,
    write_unique_backup,
};

#[cfg(test)]
mod tests {
    use std::fs::{self, Permissions};
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::{
        SetupLocation, Target, Value, eligible, ensure_hook, hook_entry, remove_hook,
        unsetup_with_remove, warn_on_replacement, write_ownership,
    };

    #[test]
    fn hook_eligibility_accepts_simple_commands_and_rejects_shell_authority() {
        for command in [
            b"git status".as_slice(),
            b"'/usr/bin/git' diff",
            b"npm test -- --runInBand",
        ] {
            assert!(eligible(command), "{command:?}");
        }
        for command in [
            b"git status | tee out".as_slice(),
            b"git $(cat command)",
            b"git status\nrm -rf x",
            b"unknown command",
            b"\"git status",
        ] {
            assert!(!eligible(command), "{command:?}");
        }
    }

    #[test]
    fn hook_mutation_preserves_unrelated_entries_and_removes_only_owned_entry() {
        let other = Value::Object(vec![(
            b"command".to_vec(),
            Value::String(b"other-hook".to_vec()),
        )]);
        let mut root = Value::Object(vec![
            (b"theme".to_vec(), Value::String(b"dark".to_vec())),
            (
                b"hooks".to_vec(),
                Value::Object(vec![(
                    b"PreToolUse".to_vec(),
                    Value::Array(vec![Value::Object(vec![(
                        b"hooks".to_vec(),
                        Value::Array(vec![other]),
                    )])]),
                )]),
            ),
        ]);
        assert!(!ensure_hook(&mut root, b"tapas-hook").unwrap());
        assert!(ensure_hook(&mut root, b"tapas-hook").unwrap());
        assert!(remove_hook(&mut root, &hook_entry(b"tapas-hook")).unwrap());
        assert_eq!(root.get(b"theme"), Some(&Value::String(b"dark".to_vec())));
        let Value::Array(entries) = root
            .get(b"hooks")
            .and_then(|hooks| hooks.get(b"PreToolUse"))
            .expect("PreToolUse handlers")
        else {
            panic!("PreToolUse is not an array");
        };
        assert!(super::nested_hook_exists(entries, b"other-hook"));
    }

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
        let sequence = super::TEMP_SEQUENCE.fetch_add(1, super::Ordering::Relaxed);
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
