use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};

use super::invocation::{Invocation, ProcessMode};
use super::spec;
use crate::catalog;

pub(super) fn run(
    invocation: Invocation<'_>,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    match invocation {
        Invocation::Version => {
            writeln!(stdout, "tapas {}", env!("TAPAS_BUILD_LABEL"))?;
            Ok(0)
        }
        Invocation::Help => {
            spec::write_help(stdout)?;
            Ok(0)
        }
        Invocation::Filters => write_filters(stdout),
        Invocation::Completions(shell) => {
            crate::completions::write(shell, stdout)?;
            Ok(0)
        }
        Invocation::RawInput if super::stdin_is_tty() => {
            usage_error(stderr, b"--raw requires a command or piped input")
        }
        Invocation::RawInput => {
            io::copy(stdin, stdout)?;
            Ok(0)
        }
        Invocation::Process { command, mode } => run_process(command, mode, stdout, stderr),
        Invocation::Rewrite(command) => rewrite(command, stdout),
        Invocation::HookEval { target, self_check } => {
            crate::setup::hook_eval_for_target(target, stdin, stdout, stderr, self_check)
        }
        Invocation::Setup(request) => crate::setup::configure_request(request, stdout, stderr),
        Invocation::Plugin(action) => match crate::plugins::manage(action, stdout) {
            Ok(code) => Ok(code),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidInput
                        | io::ErrorKind::NotFound
                        | io::ErrorKind::AlreadyExists
                        | io::ErrorKind::PermissionDenied
                        | io::ErrorKind::InvalidData
                ) =>
            {
                writeln!(stderr, "tapas: plugin: {error}")?;
                Ok(2)
            }
            Err(error) => Err(error),
        },
        Invocation::Deferred(flag) => {
            stderr.write_all(b"tapas: option ")?;
            write!(stderr, "{flag:?}")?;
            stderr.write_all(b" is not available in Tapas 0.3.0\n\n")?;
            spec::write_help(stderr)?;
            Ok(2)
        }
        Invocation::UnknownOption(option) => {
            stderr.write_all(b"tapas: unknown option ")?;
            write!(stderr, "{option:?}")?;
            stderr.write_all(b"\n\n")?;
            spec::write_help(stderr)?;
            Ok(2)
        }
        Invocation::UsageError(explanation) => usage_error(stderr, explanation),
    }
}

fn write_filters(stdout: &mut dyn Write) -> io::Result<i32> {
    writeln!(stdout, "tapas filters")?;
    writeln!(stdout)?;
    writeln!(
        stdout,
        "Auto-wrap: {}",
        catalog::AUTO_WRAP_COMMANDS.join("|")
    )?;
    writeln!(stdout)?;
    writeln!(
        stdout,
        "Transparent runners: {}",
        catalog::TRANSPARENT_RUNNERS.join("|")
    )?;
    writeln!(stdout)?;
    writeln!(
        stdout,
        "Compact routes: {}",
        catalog::COMPACT_ROUTES.join("|")
    )?;
    writeln!(stdout)?;
    writeln!(
        stdout,
        "Exact-output policies: {}",
        catalog::EXACT_OUTPUT_BYPASSES.join("|")
    )?;
    writeln!(stdout)?;
    writeln!(
        stdout,
        "Inherited/stream policies: {}",
        catalog::STREAM_WATCH_POLICIES.join("|")
    )?;
    Ok(0)
}

fn run_process(
    command: &[OsString],
    mode: ProcessMode,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    let options = match mode {
        ProcessMode::Compact => crate::process::RunOptions::default(),
        ProcessMode::Raw => crate::process::RunOptions {
            raw: true,
            explain: false,
        },
        ProcessMode::Explain => crate::process::RunOptions {
            raw: false,
            explain: true,
        },
    };
    crate::process::run(command, stdout, stderr, options).map(|report| report.exit_code)
}

fn rewrite(command: &[OsString], stdout: &mut dyn Write) -> io::Result<i32> {
    if should_wrap(command) {
        stdout.write_all(b"tapas ")?;
    }
    write_shell_command(stdout, command)?;
    stdout.write_all(b"\n")?;
    Ok(0)
}

fn usage_error(stderr: &mut dyn Write, explanation: &[u8]) -> io::Result<i32> {
    stderr.write_all(b"tapas: ")?;
    stderr.write_all(explanation)?;
    stderr.write_all(b"\n\n")?;
    spec::write_help(stderr)?;
    Ok(2)
}

fn should_wrap(command: &[OsString]) -> bool {
    let Some(program) = command.first() else {
        return false;
    };
    let basename = catalog::command_basename(program);
    basename != Some(OsStr::new("tapas")) && crate::process::invocation::is_supported(command)
}

fn write_shell_command(stdout: &mut dyn Write, command: &[OsString]) -> io::Result<()> {
    for (index, argument) in command.iter().enumerate() {
        if index != 0 {
            stdout.write_all(b" ")?;
        }
        write_shell_escaped(stdout, argument)?;
    }
    Ok(())
}

fn write_shell_escaped(stdout: &mut dyn Write, argument: &OsStr) -> io::Result<()> {
    let bytes = argument.as_encoded_bytes();
    if !bytes.is_empty() && bytes.iter().copied().all(is_shell_safe) {
        return stdout.write_all(bytes);
    }

    stdout.write_all(b"'")?;
    for byte in bytes {
        if *byte == b'\'' {
            stdout.write_all(b"'\\''")?;
        } else {
            stdout.write_all(std::slice::from_ref(byte))?;
        }
    }
    stdout.write_all(b"'")
}

fn is_shell_safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'_' | b'-' | b'.' | b'/' | b':' | b'=' | b',' | b'+' | b'@' | b'%'
        )
}
