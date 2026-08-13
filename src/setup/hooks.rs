pub(super) fn parse_config(
    existing: Option<&[u8]>,
    config_name: &str,
    stderr: &mut dyn Write,
) -> io::Result<Value> {
    let input = existing.unwrap_or(b"{}");
    if existing.is_some()
        && (input.is_empty()
            || input.iter().all(u8::is_ascii_whitespace)
            || input.starts_with(&[0xef, 0xbb, 0xbf])
            || std::str::from_utf8(input).is_err())
    {
        writeln!(stderr, "{config_name}: invalid JSON")?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid {config_name}"),
        ));
    }
    match json::parse(input) {
        Ok(value @ Value::Object(_)) => Ok(value),
        _ => {
            writeln!(stderr, "{config_name}: invalid JSON")?;
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid {config_name}"),
            ))
        }
    }
}

#[cfg(test)]
pub(super) fn ensure_hook(root: &mut Value, hook_command: &[u8]) -> Result<bool, ()> {
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
    let entry = hook_entry(hook_command);
    if entries.contains(&entry) {
        return Ok(true);
    }
    entries.push(entry);
    Ok(false)
}

pub(super) fn hook_entry(hook_command: &[u8]) -> Value {
    Value::Object(vec![
        (b"matcher".to_vec(), Value::String(b"Bash".to_vec())),
        (
            b"hooks".to_vec(),
            Value::Array(vec![Value::Object(vec![
                (b"type".to_vec(), Value::String(b"command".to_vec())),
                (b"command".to_vec(), Value::String(hook_command.to_vec())),
                (b"timeout".to_vec(), Value::Number(b"10".to_vec())),
            ])]),
        ),
    ])
}

pub(super) fn hook_exists(root: &Value, owned_entry: &Value) -> Result<bool, ()> {
    let Some(hooks) = root.get(b"hooks") else {
        return Ok(false);
    };
    if !matches!(hooks, Value::Object(_)) {
        return Err(());
    }
    let Some(events) = hooks.get(b"PreToolUse") else {
        return Ok(false);
    };
    let Value::Array(entries) = events else {
        return Err(());
    };
    Ok(entries.contains(owned_entry))
}

#[cfg(test)]
pub(super) fn nested_hook_exists(entries: &[Value], hook_command: &[u8]) -> bool {
    entries.iter().any(|entry| {
        let Some(Value::Array(handlers)) = entry.get(b"hooks") else {
            return false;
        };
        handlers.iter().any(|handler| {
            matches!(handler.get(b"command"), Some(Value::String(command)) if command == hook_command)
        })
    })
}

#[cfg(test)]
pub(super) fn remove_hook(root: &mut Value, owned_entry: &Value) -> Result<bool, ()> {
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
    let Some(position) = entries.iter().position(|entry| entry == owned_entry) else {
        return Ok(false);
    };
    entries.remove(position);
    Ok(true)
}

pub(super) fn event_command(value: &Value) -> Option<&[u8]> {
    let Value::String(command) = value.get(b"tool_input")?.get(b"command")? else {
        return None;
    };
    Some(command)
}

pub(super) fn eligible(command: &[u8]) -> bool {
    let Some(words) = shell_words(command) else {
        return false;
    };
    let argv = words
        .into_iter()
        .map(OsString::from_vec)
        .collect::<Vec<_>>();
    crate::process::invocation::is_supported(&argv)
}

pub(super) fn codex_read_only(command: &[u8]) -> bool {
    let Some(words) = shell_words(command) else {
        return false;
    };
    codex_words_read_only(&words)
}

pub(super) struct CodexCommand {
    pub environment: &'static [u8],
    pub command: Vec<u8>,
}

pub(super) fn codex_command(command: &[u8], cwd: &[u8]) -> Option<CodexCommand> {
    let mut words = shell_words(command)?;
    if !codex_words_read_only(&words) {
        return None;
    }
    let program = words[0].clone();
    let resolved = resolve_trusted_program(&program, cwd)?;
    words[0] = resolved.as_os_str().as_bytes().to_vec();
    let environment = match program.as_slice() {
        b"rg" => {
            words.insert(1, b"--no-config".to_vec());
            b"".as_slice()
        }
        b"git" => {
            let subcommand =
                usize::from(words.get(1).is_some_and(|word| word == b"--no-pager")) + 1;
            if matches!(
                words.get(subcommand).map(Vec::as_slice),
                Some(b"diff" | b"log" | b"show")
            ) {
                words.insert(subcommand + 1, b"--no-ext-diff".to_vec());
                words.insert(subcommand + 2, b"--no-textconv".to_vec());
            }
            b"GIT_OPTIONAL_LOCKS=0 GIT_CONFIG_COUNT=3 GIT_CONFIG_KEY_0=core.fsmonitor GIT_CONFIG_VALUE_0=false GIT_CONFIG_KEY_1=log.showSignature GIT_CONFIG_VALUE_1=false GIT_CONFIG_KEY_2=format.pretty GIT_CONFIG_VALUE_2=medium GIT_PAGER= "
                .as_slice()
        }
        _ => b"".as_slice(),
    };
    Some(CodexCommand {
        environment,
        command: shell_join(&words),
    })
}

fn codex_words_read_only(words: &[Vec<u8>]) -> bool {
    let program = words[0].as_slice();
    if program.contains(&b'/') {
        return false;
    }
    let arguments = &words[1..];
    match program {
        b"cat" | b"du" | b"jq" | b"ls" | b"ps" => true,
        b"tree" => !arguments.iter().any(|argument| {
            argument.starts_with(b"-o")
                || argument == b"--output"
                || argument.starts_with(b"--output=")
        }),
        b"rg" => !arguments.iter().any(|argument| {
            matches!(
                argument.as_slice(),
                b"--hostname-bin" | b"--pre" | b"--pre-glob" | b"--search-zip" | b"-z"
            ) || argument.starts_with(b"--hostname-bin=")
                || argument.starts_with(b"--pre=")
                || argument.starts_with(b"--pre-glob=")
        }),
        b"find" => !arguments.iter().any(|argument| {
            argument == b"-delete"
                || argument.starts_with(b"-exec")
                || argument.starts_with(b"-fls")
                || argument.starts_with(b"-fprint")
                || argument.starts_with(b"-ok")
        }),
        b"git" => git_read_only(arguments),
        _ => false,
    }
}

fn resolve_trusted_program(program: &[u8], cwd: &[u8]) -> Option<PathBuf> {
    let cwd = fs::canonicalize(Path::new(OsStr::from_bytes(cwd))).ok()?;
    let workspace = cwd
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(&cwd);
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(OsStr::from_bytes(program));
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            continue;
        }
        let resolved = fs::canonicalize(candidate).ok()?;
        if resolved.starts_with(workspace) || path_is_writable_by_others(&resolved) {
            continue;
        }
        return Some(resolved);
    }
    None
}

fn path_is_writable_by_others(path: &Path) -> bool {
    path.ancestors()
        .any(|ancestor| match fs::metadata(ancestor) {
            Ok(metadata) => metadata.permissions().mode() & 0o022 != 0,
            Err(_) => true,
        })
}

fn shell_join(words: &[Vec<u8>]) -> Vec<u8> {
    let mut command = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if index != 0 {
            command.push(b' ');
        }
        command.extend_from_slice(&shell_escape(OsStr::from_bytes(word)));
    }
    command
}

fn git_read_only(arguments: &[Vec<u8>]) -> bool {
    let (subcommand, rest) = match arguments {
        [option, subcommand, rest @ ..] if option == b"--no-pager" => (subcommand.as_slice(), rest),
        [subcommand, rest @ ..] if !subcommand.starts_with(b"-") => (subcommand.as_slice(), rest),
        _ => return false,
    };
    matches!(
        subcommand,
        b"blame" | b"diff" | b"log" | b"show" | b"status"
    ) && !rest.iter().any(|argument| {
        matches!(
            argument.as_slice(),
            b"--ext-diff" | b"--textconv" | b"--show-signature"
        ) || argument.starts_with(b"--ext-diff=")
            || argument.starts_with(b"--textconv=")
            || argument == b"--pretty"
            || argument.starts_with(b"--pretty=")
            || argument.windows(2).any(|window| window == b"%G")
            || argument == b"--output"
            || argument.starts_with(b"--output=")
    })
}

fn shell_words(command: &[u8]) -> Option<Vec<Vec<u8>>> {
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Quote {
        Unquoted,
        Single,
        Double,
    }

    let mut words = Vec::new();
    let mut word = Vec::new();
    let mut quote = Quote::Unquoted;
    let mut started = false;
    let mut index = 0usize;
    while index < command.len() {
        let byte = command[index];
        if matches!(byte, b'\n' | b'\r') {
            return None;
        }
        match quote {
            Quote::Single => {
                if byte == b'\'' {
                    quote = Quote::Unquoted;
                } else {
                    push_word_byte(&words, &mut word, byte)?;
                }
            }
            Quote::Double => {
                if byte == b'"' {
                    quote = Quote::Unquoted;
                } else if matches!(byte, b'`' | b'$') {
                    return None;
                } else if byte == b'\\' {
                    index += 1;
                    let escaped = command.get(index).copied()?;
                    if matches!(escaped, b'\n' | b'\r') {
                        return None;
                    }
                    push_word_byte(&words, &mut word, escaped)?;
                } else {
                    push_word_byte(&words, &mut word, byte)?;
                }
            }
            Quote::Unquoted => {
                if byte.is_ascii_whitespace() {
                    if started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                } else if b";|&<>`(){}*?[]~#".contains(&byte) || byte == b'$' {
                    return None;
                } else if byte == b'\'' {
                    started = true;
                    quote = Quote::Single;
                } else if byte == b'"' {
                    started = true;
                    quote = Quote::Double;
                } else if byte == b'\\' {
                    index += 1;
                    let escaped = command.get(index).copied()?;
                    if matches!(escaped, b'\n' | b'\r') {
                        return None;
                    }
                    started = true;
                    push_word_byte(&words, &mut word, escaped)?;
                } else {
                    started = true;
                    push_word_byte(&words, &mut word, byte)?;
                }
            }
        }
        index += 1;
    }
    if quote != Quote::Unquoted {
        return None;
    }
    if started {
        words.push(word);
    }
    (!words.is_empty()).then_some(words)
}

fn push_word_byte(words: &[Vec<u8>], word: &mut Vec<u8>, byte: u8) -> Option<()> {
    if words.is_empty() && word.len() == 256 {
        return None;
    }
    word.push(byte);
    Some(())
}

pub(super) fn hook_command(executable: &OsStr, target: Target) -> Vec<u8> {
    let mut command = shell_escape(executable);
    command.extend_from_slice(b" --hook-eval ");
    command.extend_from_slice(target.name().as_bytes());
    command
}

pub(super) fn shell_escape(value: &OsStr) -> Vec<u8> {
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

pub(super) fn validate_hook(executable: &Path, target: Target) -> io::Result<bool> {
    let output = Command::new(executable)
        .args(["--hook-eval", target.name(), "--self-check"])
        .stdin(Stdio::null())
        .output()?;
    Ok(output.status.success() && output.stdout.is_empty() && output.stderr.is_empty())
}

pub(super) fn contains_conflicting_integration(input: &[u8]) -> bool {
    match json::parse(input) {
        Ok(value) => json_integration_conflict(&value),
        Err(_) => toml_hook_conflict(input),
    }
}

fn json_integration_conflict(value: &Value) -> bool {
    let Value::Object(fields) = value else {
        return false;
    };
    fields.iter().any(|(key, value)| {
        if matches!(key.as_slice(), b"command" | b"plugin" | b"plugins") {
            return integration_value_conflict(value);
        }
        matches!(key.as_slice(), b"hooks" | b"PreToolUse") && json_integration_conflict(value)
            || matches!(value, Value::Array(items) if items.iter().any(json_integration_conflict))
    })
}

fn integration_value_conflict(value: &Value) -> bool {
    match value {
        Value::String(text) => predecessor_command(text),
        Value::Array(items) => items.iter().any(integration_value_conflict),
        Value::Object(_) => json_integration_conflict(value),
        _ => false,
    }
}

fn predecessor_command(input: &[u8]) -> bool {
    [b"run-toolkit".as_slice(), b"smll", b"rtk"]
        .iter()
        .any(|name| {
            input
                .split(|byte| byte.is_ascii_whitespace() || b"/'\"=,:[]()".contains(byte))
                .any(|word| word.eq_ignore_ascii_case(name))
        })
}

fn toml_hook_conflict(input: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(input) else {
        return false;
    };
    let mut hooks = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            hooks = line.trim_matches(|character| matches!(character, '[' | ']' | ' ')) == "hooks";
            continue;
        }
        if hooks
            && line.split_once('=').is_some_and(|(key, value)| {
                key.trim() == "command" && predecessor_command(value.as_bytes())
            })
        {
            return true;
        }
    }
    false
}
use super::{Target, Value, json};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
