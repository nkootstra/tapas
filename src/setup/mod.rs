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
const OWNERSHIP_HEADER: &[u8] = b"tapas-setup-v2\n";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub enum Action {
    Setup,
    Unsetup,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Target {
    Claude,
    Codex,
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
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn config_path(self, home: &Path, codex_home: Option<&Path>) -> std::path::PathBuf {
        match self {
            Self::Claude => home.join(".claude").join(self.config_name()),
            Self::Codex => codex_home
                .map_or_else(|| home.join(".codex"), Path::to_path_buf)
                .join(self.config_name()),
        }
    }

    fn config_name(self) -> &'static str {
        match self {
            Self::Claude => "settings.json",
            Self::Codex => "hooks.json",
        }
    }

    fn grants_rewrite_permission(self) -> bool {
        self == Self::Codex
    }

    fn accepts_hook_event(self, value: &Value) -> bool {
        self == Self::Claude
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
        return Ok(i32::from(!eligible(b"git status")));
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
    if !eligible(command) {
        return Ok(0);
    }

    let mut updated_input = value
        .get(b"tool_input")
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "tool_input disappeared"))?;
    let executable = std::env::current_exe()?;
    let mut updated_command = shell_escape(executable.as_os_str());
    updated_command.push(b' ');
    updated_command.extend_from_slice(command);
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
    let location = SetupLocation {
        config_path: target.config_path(home, codex_home.as_deref()),
        ownership_path: home.join(format!(".tapas/setup/{}.owned", target.name())),
        target,
    };
    configure_at(&location, &executable, action, dry_run, stdout, stderr)
}

fn configure_at(
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
    if reject_symlink(config_path, stderr)? {
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
    let mut root = match parse_config(existing.as_deref(), target.config_name(), stderr) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(1),
        Err(error) => return Err(error),
    };
    let ownership = read_ownership(ownership_path)?;
    let owned_entry = match ownership {
        Ownership::Modified => {
            stderr.write_all(
                b"tapas setup ownership record was modified; configuration left untouched\n",
            )?;
            return Ok(1);
        }
        Ownership::Valid(ref entry) => Some(entry),
        Ownership::Missing => None,
    };

    let expected_entry = hook_entry(hook_command);
    let removed_owned = match owned_entry {
        Some(owned) if owned == &expected_entry => match hook_exists(&root, owned) {
            Ok(true) => false,
            Ok(false) => {
                stderr.write_all(b"tapas-owned hook entry was modified or removed; configuration left untouched\n")?;
                return Ok(1);
            }
            Err(()) => {
                writeln!(stderr, "{}: invalid JSON", target.config_name())?;
                return Ok(1);
            }
        },
        Some(owned) => match remove_hook(&mut root, owned) {
            Ok(true) => true,
            Ok(false) => {
                stderr.write_all(b"tapas-owned hook entry was modified or removed; configuration left untouched\n")?;
                return Ok(1);
            }
            Err(()) => {
                writeln!(stderr, "{}: invalid JSON", target.config_name())?;
                return Ok(1);
            }
        },
        None => false,
    };
    let already_installed = match ensure_hook(&mut root, hook_command) {
        Ok(installed) => installed,
        Err(()) => {
            writeln!(stderr, "{}: invalid JSON", target.config_name())?;
            return Ok(1);
        }
    };
    let changed = removed_owned || !already_installed;
    if changed {
        if dry_run {
            writeln!(stdout, "[dry-run] would update {}", config_path.display())?;
        } else {
            write_backup(config_path, existing.as_deref())?;
            let mut rendered = json::serialize(&root);
            rendered.push(b'\n');
            write_atomic(config_path, &rendered, existing_mode(config_path, 0o600))?;
            writeln!(stdout, "updated {}", config_path.display())?;
        }
    } else {
        stdout.write_all(b"already installed\n")?;
    }

    if dry_run {
        stdout.write_all(b"[dry-run] would record tapas hook ownership\n")?;
        return Ok(0);
    }
    if let Err(error) = write_ownership(ownership_path, &expected_entry) {
        if changed {
            restore_optional(config_path, existing.as_deref())?;
            remove_backup(config_path)?;
        }
        return Err(error);
    }
    stdout.write_all(b"ok\n")?;
    Ok(0)
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
    if reject_symlink(config_path, stderr)? {
        return Ok(1);
    }
    let owned_entry = match read_ownership(ownership_path)? {
        Ownership::Missing => {
            stderr.write_all(b"tapas hook ownership record not found; no hook was removed\n")?;
            return Ok(0);
        }
        Ownership::Modified => {
            stderr.write_all(b"tapas hook ownership record was modified; no hook was removed\n")?;
            return Ok(0);
        }
        Ownership::Valid(command) => command,
    };
    let Some(existing) = read_optional(config_path, MAX_CONFIG_BYTES)? else {
        stdout.write_all(b"not found\n")?;
        return Ok(0);
    };
    let mut root = match parse_config(Some(&existing), target.config_name(), stderr) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(1),
        Err(error) => return Err(error),
    };
    let removed = match remove_hook(&mut root, &owned_entry) {
        Ok(removed) => removed,
        Err(()) => {
            writeln!(stderr, "{}: invalid JSON", target.config_name())?;
            return Ok(1);
        }
    };
    if !removed {
        writeln!(
            stderr,
            "tapas-owned hook entry was modified or removed; configuration left untouched; rerun tapas --setup {} to recover",
            target.name()
        )?;
        return Ok(0);
    }
    if dry_run {
        writeln!(stdout, "[dry-run] would update {}", config_path.display())?;
        return Ok(0);
    }

    let original_mode = existing_mode(config_path, 0o600);
    write_backup(config_path, Some(&existing))?;
    let mut rendered = json::serialize(&root);
    rendered.push(b'\n');
    write_atomic(config_path, &rendered, original_mode)?;
    if let Err(remove_error) = remove_ownership(ownership_path) {
        if let Err(rollback_error) = write_atomic(config_path, &existing, original_mode) {
            return Err(io::Error::new(
                rollback_error.kind(),
                format!(
                    "failed to remove ownership record ({remove_error}); settings rollback failed: {rollback_error}"
                ),
            ));
        }
        return Err(remove_error);
    }
    writeln!(stdout, "updated {}", config_path.display())?;
    stdout.write_all(b"ok\n")?;
    Ok(0)
}

fn reject_symlink(path: &Path, stderr: &mut dyn Write) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            stderr.write_all(b"tapas agent setup: symbolic-link configuration is not supported; configuration left untouched\n")?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

mod hooks;
mod ownership;
mod storage;

#[cfg(test)]
use hooks::nested_hook_exists;
use hooks::{
    contains_conflicting_integration, eligible, ensure_hook, event_command, hook_command,
    hook_entry, hook_exists, parse_config, remove_hook, shell_escape, validate_hook,
};
use ownership::{Ownership, read_ownership, write_ownership};
use storage::{
    existing_mode, read_optional, remove_backup, restore_optional, write_atomic, write_backup,
};

#[cfg(test)]
mod tests {
    use std::fs::{self, Permissions};
    use std::io;
    use std::os::unix::fs::PermissionsExt;

    use super::{
        SetupLocation, Target, Value, eligible, ensure_hook, hook_entry, remove_hook,
        unsetup_with_remove, write_ownership,
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
