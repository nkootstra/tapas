mod json;

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use json::Value;

const MAX_HOOK_INPUT: u64 = 64 * 1024;
const MAX_CONFIG_BYTES: u64 = 8 * 1024 * 1024;
const OWNERSHIP_HEADER: &[u8] = b"tapas-setup-v1\n";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub enum Action {
    Setup,
    Unsetup,
}

pub fn hook_eval(
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
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
    let Some(command) = event_command(&value) else {
        return Ok(0);
    };
    if !eligible(command) {
        return Ok(0);
    }
    stderr.write_all(b"tapas hook: wrap noisy command with tapas (example: tapas ")?;
    stderr.write_all(command)?;
    stderr.write_all(b")\n")?;
    let _ = stdout;
    Ok(2)
}

pub fn configure(
    action: Action,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    let Some(home) = std::env::var_os("HOME") else {
        stderr.write_all(b"tapas agent setup: HOME is not set\n")?;
        return Ok(1);
    };
    let executable = std::env::current_exe()?;
    configure_at(
        Path::new(&home),
        &executable,
        action,
        dry_run,
        true,
        stdout,
        stderr,
    )
}

fn configure_at(
    home: &Path,
    executable: &Path,
    action: Action,
    dry_run: bool,
    validate: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    let config_path = home.join(".claude/settings.json");
    let ownership_path = home.join(".tapas/setup/claude.owned");
    let hook_command = hook_command(executable.as_os_str());
    match action {
        Action::Setup => setup(
            &config_path,
            &ownership_path,
            executable,
            &hook_command,
            dry_run,
            validate,
            stdout,
            stderr,
        ),
        Action::Unsetup => unsetup(&config_path, &ownership_path, dry_run, stdout, stderr),
    }
}

#[allow(clippy::too_many_arguments)]
fn setup(
    config_path: &Path,
    ownership_path: &Path,
    executable: &Path,
    hook_command: &[u8],
    dry_run: bool,
    validate: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    if validate && !validate_hook(executable)? {
        stderr.write_all(b"tapas hook evaluator self-check failed\n")?;
        return Ok(1);
    }
    let existing = read_optional(config_path, MAX_CONFIG_BYTES)?;
    if existing
        .as_deref()
        .is_some_and(contains_conflicting_integration)
    {
        stderr.write_all(b"Conflicting command-wrapper integration detected in settings.json. Remove it first, then run tapas --setup claude again.\n")?;
        return Ok(1);
    }
    let mut root = match parse_config(existing.as_deref(), stderr) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(1),
        Err(error) => return Err(error),
    };
    let ownership = read_ownership(ownership_path)?;
    let owned_command = match ownership {
        Ownership::Modified => {
            stderr.write_all(
                b"tapas setup ownership record was modified; configuration left untouched\n",
            )?;
            return Ok(1);
        }
        Ownership::Valid(ref command) => Some(command.as_slice()),
        Ownership::Missing => None,
    };

    let removed_owned = owned_command
        .filter(|owned| *owned != hook_command)
        .is_some_and(|owned| remove_hook(&mut root, owned).unwrap_or(false));
    let already_installed = match ensure_hook(&mut root, hook_command) {
        Ok(installed) => installed,
        Err(()) => {
            stderr.write_all(b"settings.json: invalid JSON\n")?;
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
    if let Err(error) = write_ownership(ownership_path, hook_command) {
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
    config_path: &Path,
    ownership_path: &Path,
    dry_run: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    let owned_command = match read_ownership(ownership_path)? {
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
    let mut root = match parse_config(Some(&existing), stderr) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(1),
        Err(error) => return Err(error),
    };
    let removed = match remove_hook(&mut root, &owned_command) {
        Ok(removed) => removed,
        Err(()) => {
            stderr.write_all(b"settings.json: invalid JSON\n")?;
            return Ok(1);
        }
    };
    if !removed {
        stderr.write_all(b"tapas-owned hook entry was modified or removed; configuration left untouched; rerun tapas --setup claude to recover\n")?;
        return Ok(0);
    }
    if dry_run {
        writeln!(stdout, "[dry-run] would update {}", config_path.display())?;
        return Ok(0);
    }

    write_backup(config_path, Some(&existing))?;
    let mut rendered = json::serialize(&root);
    rendered.push(b'\n');
    write_atomic(config_path, &rendered, existing_mode(config_path, 0o600))?;
    writeln!(stdout, "updated {}", config_path.display())?;
    fs::remove_file(ownership_path)?;
    stdout.write_all(b"ok\n")?;
    Ok(0)
}

fn parse_config(existing: Option<&[u8]>, stderr: &mut dyn Write) -> io::Result<Value> {
    let input = existing
        .filter(|bytes| !bytes.iter().all(u8::is_ascii_whitespace))
        .unwrap_or(b"{}");
    match json::parse(input) {
        Ok(value @ Value::Object(_)) => Ok(value),
        _ => {
            stderr.write_all(b"settings.json: invalid JSON\n")?;
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid settings.json",
            ))
        }
    }
}

fn ensure_hook(root: &mut Value, hook_command: &[u8]) -> Result<bool, ()> {
    if root.get(b"hooks").is_none() {
        root.insert(b"hooks", Value::object()).map_err(|_| ())?;
    }
    let hooks = root.get_mut(b"hooks").ok_or(())?;
    if !matches!(hooks, Value::Object(_)) {
        return Err(());
    }
    if hooks.get(b"PreToolUse").is_none() {
        hooks
            .insert(b"PreToolUse", Value::Array(Vec::new()))
            .map_err(|_| ())?;
    }
    let Value::Array(entries) = hooks.get_mut(b"PreToolUse").ok_or(())? else {
        return Err(());
    };
    if nested_hook_exists(entries, hook_command) {
        return Ok(true);
    }
    entries.push(Value::Object(vec![
        (b"matcher".to_vec(), Value::String(b"Bash".to_vec())),
        (
            b"hooks".to_vec(),
            Value::Array(vec![Value::Object(vec![
                (b"type".to_vec(), Value::String(b"command".to_vec())),
                (b"command".to_vec(), Value::String(hook_command.to_vec())),
                (b"timeout".to_vec(), Value::Integer(10)),
            ])]),
        ),
    ]));
    Ok(false)
}

fn nested_hook_exists(entries: &[Value], hook_command: &[u8]) -> bool {
    entries.iter().any(|entry| {
        let Some(Value::Array(handlers)) = entry.get(b"hooks") else {
            return false;
        };
        handlers.iter().any(|handler| {
            matches!(handler.get(b"command"), Some(Value::String(command)) if command == hook_command)
        })
    })
}

fn remove_hook(root: &mut Value, hook_command: &[u8]) -> Result<bool, ()> {
    let Some(hooks) = root.get_mut(b"hooks") else {
        return Ok(false);
    };
    if !matches!(hooks, Value::Object(_)) {
        return Err(());
    }
    let Some(events) = hooks.get_mut(b"PreToolUse") else {
        return Ok(false);
    };
    let Value::Array(entries) = events else {
        return Err(());
    };
    let mut removed = false;
    for entry in entries.iter_mut() {
        let Some(handlers) = entry.get_mut(b"hooks") else {
            continue;
        };
        let Value::Array(handlers) = handlers else {
            continue;
        };
        handlers.retain(|handler| {
            let owned = matches!(handler.get(b"command"), Some(Value::String(command)) if command == hook_command);
            removed |= owned;
            !owned
        });
    }
    entries.retain(
        |entry| !matches!(entry.get(b"hooks"), Some(Value::Array(handlers)) if handlers.is_empty()),
    );
    Ok(removed)
}

fn event_command(value: &Value) -> Option<&[u8]> {
    let Value::String(command) = value.get(b"tool_input")?.get(b"command")? else {
        return None;
    };
    Some(command)
}

fn eligible(command: &[u8]) -> bool {
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Quote {
        Unquoted,
        Single,
        Double,
    }

    let mut token = [0_u8; 256];
    let mut token_len = 0usize;
    let mut quote = Quote::Unquoted;
    let mut started = false;
    let mut finished = false;
    let mut index = 0usize;
    while index < command.len() {
        let byte = command[index];
        if matches!(byte, b'\n' | b'\r') {
            return false;
        }
        match quote {
            Quote::Single => {
                if byte == b'\'' {
                    quote = Quote::Unquoted;
                } else if !finished && !push_token(&mut token, &mut token_len, byte) {
                    return false;
                }
            }
            Quote::Double => {
                if byte == b'"' {
                    quote = Quote::Unquoted;
                } else if matches!(byte, b'`' | b'$') {
                    return false;
                } else if byte == b'\\' {
                    index += 1;
                    let Some(escaped) = command.get(index).copied() else {
                        return false;
                    };
                    if matches!(escaped, b'\n' | b'\r')
                        || !finished && !push_token(&mut token, &mut token_len, escaped)
                    {
                        return false;
                    }
                } else if !finished && !push_token(&mut token, &mut token_len, byte) {
                    return false;
                }
            }
            Quote::Unquoted => {
                if byte.is_ascii_whitespace() {
                    if started {
                        finished = true;
                    }
                } else if b";|&<>`(){}*?[]~#".contains(&byte) || byte == b'$' {
                    return false;
                } else if byte == b'\'' {
                    if !finished {
                        started = true;
                    }
                    quote = Quote::Single;
                } else if byte == b'"' {
                    if !finished {
                        started = true;
                    }
                    quote = Quote::Double;
                } else if byte == b'\\' {
                    index += 1;
                    let Some(escaped) = command.get(index).copied() else {
                        return false;
                    };
                    if matches!(escaped, b'\n' | b'\r') {
                        return false;
                    }
                    if !finished {
                        started = true;
                        if !push_token(&mut token, &mut token_len, escaped) {
                            return false;
                        }
                    }
                } else if !finished {
                    started = true;
                    if !push_token(&mut token, &mut token_len, byte) {
                        return false;
                    }
                }
            }
        }
        index += 1;
    }
    if quote != Quote::Unquoted || token_len == 0 {
        return false;
    }
    let first = &token[..token_len];
    let basename = first
        .iter()
        .rposition(|byte| *byte == b'/')
        .map_or(first, |slash| &first[slash + 1..]);
    crate::catalog::AUTO_WRAP_COMMANDS
        .iter()
        .any(|candidate| candidate.as_bytes() == basename)
}

fn push_token(token: &mut [u8; 256], length: &mut usize, byte: u8) -> bool {
    if *length == token.len() {
        return false;
    }
    token[*length] = byte;
    *length += 1;
    true
}

fn hook_command(executable: &OsStr) -> Vec<u8> {
    let mut command = shell_escape(executable);
    command.extend_from_slice(b" --hook-eval claude");
    command
}

fn shell_escape(value: &OsStr) -> Vec<u8> {
    let mut output = Vec::with_capacity(value.as_bytes().len() + 2);
    output.push(b'\'');
    for byte in value.as_bytes() {
        if *byte == b'\'' {
            output.extend_from_slice(b"'\\''");
        } else {
            output.push(*byte);
        }
    }
    output.push(b'\'');
    output
}

fn validate_hook(executable: &Path) -> io::Result<bool> {
    let output = Command::new(executable)
        .args(["--hook-eval", "claude", "--self-check"])
        .stdin(Stdio::null())
        .output()?;
    Ok(output.status.success() && output.stdout.is_empty() && output.stderr.is_empty())
}

fn contains_conflicting_integration(input: &[u8]) -> bool {
    [
        b"run-toolkit".as_slice(),
        b"run toolkit",
        b"\"rtk\"",
        b" rtk",
        b"/rtk",
        b"rtk-",
        b"-rtk",
    ]
    .iter()
    .any(|needle| contains_ignore_ascii_case(input, needle))
}

fn contains_ignore_ascii_case(input: &[u8], needle: &[u8]) -> bool {
    input.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

enum Ownership {
    Missing,
    Modified,
    Valid(Vec<u8>),
}

fn read_ownership(path: &Path) -> io::Result<Ownership> {
    let Some(content) = read_optional(path, MAX_CONFIG_BYTES)? else {
        return Ok(Ownership::Missing);
    };
    let Some(rest) = content.strip_prefix(OWNERSHIP_HEADER) else {
        return Ok(Ownership::Modified);
    };
    let Some(newline) = rest.iter().position(|byte| *byte == b'\n') else {
        return Ok(Ownership::Modified);
    };
    if newline != 16 {
        return Ok(Ownership::Modified);
    }
    let payload = &rest[newline + 1..];
    if rest[..newline] != digest(payload) {
        return Ok(Ownership::Modified);
    }
    Ok(Ownership::Valid(payload.to_vec()))
}

fn write_ownership(path: &Path, payload: &[u8]) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ownership path has no parent",
        ));
    };
    fs::create_dir_all(parent)?;
    if let Some(root) = parent.parent() {
        fs::set_permissions(root, Permissions::from_mode(0o700))?;
    }
    fs::set_permissions(parent, Permissions::from_mode(0o700))?;
    let mut content = OWNERSHIP_HEADER.to_vec();
    content.extend_from_slice(&digest(payload));
    content.push(b'\n');
    content.extend_from_slice(payload);
    write_atomic(path, &content, 0o600)
}

fn digest(input: &[u8]) -> [u8; 16] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for byte in input {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100_0000_01b3);
    }
    let mut output = [0_u8; 16];
    for index in (0..output.len()).rev() {
        output[index] = HEX[(value & 0x0f) as usize];
        value >>= 4;
    }
    output
}

fn read_optional(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    let mut content = Vec::new();
    file.take(limit + 1).read_to_end(&mut content)?;
    if content.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    Ok(Some(content))
}

fn write_backup(path: &Path, existing: Option<&[u8]>) -> io::Result<()> {
    let Some(existing) = existing else {
        return Ok(());
    };
    write_atomic(&backup_path(path), existing, existing_mode(path, 0o600))
}

fn remove_backup(path: &Path) -> io::Result<()> {
    match fs::remove_file(backup_path(path)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .unwrap_or(OsStr::new("settings.json"))
        .to_os_string();
    name.push(".bak.tapas");
    path.with_file_name(name)
}

fn restore_optional(path: &Path, existing: Option<&[u8]>) -> io::Result<()> {
    if let Some(existing) = existing {
        write_atomic(path, existing, existing_mode(path, 0o600))
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn existing_mode(path: &Path, default: u32) -> u32 {
    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o777)
        .unwrap_or(default)
}

fn write_atomic(path: &Path, content: &[u8], mode: u32) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output path has no parent"))?;
    fs::create_dir_all(parent)?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = OsString::from(".");
    temp_name.push(path.file_name().unwrap_or(OsStr::new("tapas")));
    temp_name.push(format!(".tmp.{}.{sequence}", std::process::id()));
    let temp = parent.join(temp_name);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temp)?;
        file.write_all(content)?;
        file.sync_all()?;
        fs::set_permissions(&temp, Permissions::from_mode(mode))?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{Value, eligible, ensure_hook, remove_hook};

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
    fn hook_mutation_preserves_unrelated_entries_and_removes_only_owned_command() {
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
        assert!(remove_hook(&mut root, b"tapas-hook").unwrap());
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
}
