use std::io::{self, Read, Write};

use super::Target;
use super::hooks::{codex_command, codex_read_only, eligible, event_command, shell_escape};
use super::json::{self, Value};

const MAX_HOOK_INPUT: u64 = 64 * 1024;

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
        return Ok(i32::from(!accepts_command(target, b"git status")));
    }
    let mut input = Vec::new();
    stdin.take(MAX_HOOK_INPUT + 1).read_to_end(&mut input)?;
    if input.len() as u64 > MAX_HOOK_INPUT {
        return Ok(0);
    }
    let Ok(value) = json::parse(&input) else {
        return Ok(0);
    };
    if !accepts_hook_event(target, &value) {
        return Ok(0);
    }
    let Some(command) = event_command(&value) else {
        return Ok(0);
    };
    let (environment, command) = match target {
        Target::OpenCode if accepts_command(target, command) => {
            let executable = std::env::current_exe()?;
            let mut updated = shell_escape(executable.as_os_str());
            updated.push(b' ');
            updated.extend_from_slice(command);
            stdout.write_all(&updated)?;
            stdout.write_all(b"\n")?;
            return Ok(0);
        }
        Target::Claude if accepts_command(target, command) => (b"".as_slice(), command.to_vec()),
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

fn accepts_command(target: Target, command: &[u8]) -> bool {
    eligible(command) && (target != Target::Codex || codex_read_only(command))
}

fn accepts_hook_event(target: Target, value: &Value) -> bool {
    target != Target::Codex
        || matches!(
            (value.get(b"hook_event_name"), value.get(b"tool_name")),
            (Some(Value::String(event)), Some(Value::String(tool)))
                if event == b"PreToolUse" && tool == b"Bash"
        )
}
