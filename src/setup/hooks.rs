pub(super) fn parse_config(existing: Option<&[u8]>, stderr: &mut dyn Write) -> io::Result<Value> {
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

pub(super) fn hook_command(executable: &OsStr) -> Vec<u8> {
    let mut command = shell_escape(executable);
    command.extend_from_slice(b" --hook-eval claude");
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

pub(super) fn validate_hook(executable: &Path) -> io::Result<bool> {
    let output = Command::new(executable)
        .args(["--hook-eval", "claude", "--self-check"])
        .stdin(Stdio::null())
        .output()?;
    Ok(output.status.success() && output.stdout.is_empty() && output.stderr.is_empty())
}

pub(super) fn contains_conflicting_integration(input: &[u8]) -> bool {
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
use super::{Value, json};
use crate::filters::contains_ignore_ascii_case;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::{Command, Stdio};
