use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};

use crate::catalog;

const HELP: &str = "\
Usage:
  tapas <cmd...>
  <cmd> | tapas
  tapas --raw [--] <cmd...>
  <cmd> | tapas --raw
  tapas --explain <cmd...>
  tapas --rewrite <cmd...>
  tapas --hook-eval claude
  tapas --hook-eval codex
  tapas --hook-eval opencode
  tapas --setup claude [--dry-run]
  tapas --setup codex [--dry-run]
  tapas --setup opencode [--dry-run] [--force]
  tapas --unsetup claude [--dry-run]
  tapas --unsetup codex [--dry-run]
  tapas --unsetup opencode [--dry-run]

Options:
  -h, --help       Show this help
  --version        Show the Tapas version
  --filters        List the static compatibility catalogs
";
const SETUP_USAGE: &[u8] =
    b"usage: tapas --setup <claude|codex|opencode> [--dry-run] [--force]\n       tapas --unsetup <claude|codex|opencode> [--dry-run]\n";

pub fn main_entry() -> i32 {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();

    match run(&args, &mut stdin, &mut stdout, &mut stderr) {
        Ok(status) => status,
        Err(error) => {
            let _ = writeln!(stderr, "tapas: internal I/O error: {error}");
            1
        }
    }
}

pub fn run(
    args: &[OsString],
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    match args {
        [] if stdin_is_tty() => {
            stdout.write_all(HELP.as_bytes())?;
            Ok(0)
        }
        [] if crate::environment::flag_on("TAPAS_LOSSLESS") => {
            io::copy(stdin, stdout)?;
            Ok(0)
        }
        [] => {
            crate::pipeline::run(stdin, stdout)?;
            Ok(0)
        }
        [arg] if arg == OsStr::new("--version") => {
            writeln!(stdout, "tapas {}", env!("TAPAS_BUILD_LABEL"))?;
            Ok(0)
        }
        [arg] if arg == OsStr::new("--help") || arg == OsStr::new("-h") => {
            stdout.write_all(HELP.as_bytes())?;
            Ok(0)
        }
        [arg] if arg == OsStr::new("--filters") => {
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
            Ok(0)
        }
        [arg] if arg == OsStr::new("--raw") => {
            if stdin_is_tty() {
                stderr
                    .write_all(b"usage: tapas --raw [--] <cmd...>\n       <cmd> | tapas --raw\n")?;
                return Ok(2);
            }
            io::copy(stdin, stdout)?;
            Ok(0)
        }
        [flag, rest @ ..] if flag == OsStr::new("--raw") => {
            let command = rest.strip_prefix(&[OsString::from("--")]).unwrap_or(rest);
            if command.is_empty() {
                stderr
                    .write_all(b"usage: tapas --raw [--] <cmd...>\n       <cmd> | tapas --raw\n")?;
                return Ok(2);
            }
            crate::process::run(
                command,
                stdout,
                stderr,
                crate::process::RunOptions {
                    raw: true,
                    explain: false,
                },
            )
            .map(|report| report.exit_code)
        }
        [flag, command @ ..] if flag == OsStr::new("--explain") && !command.is_empty() => {
            crate::process::run(
                command,
                stdout,
                stderr,
                crate::process::RunOptions {
                    raw: false,
                    explain: true,
                },
            )
            .map(|report| report.exit_code)
        }
        [flag, command @ ..] if flag == OsStr::new("--rewrite") && !command.is_empty() => {
            if should_wrap(command) {
                stdout.write_all(b"tapas ")?;
            }
            write_shell_command(stdout, command)?;
            stdout.write_all(b"\n")?;
            Ok(0)
        }
        [flag, ..] if flag == OsStr::new("--hook-eval") => {
            let Some((target, self_check)) = hook_request(args) else {
                stderr.write_all(b"usage: tapas --hook-eval <claude|codex|opencode>\n")?;
                return Ok(2);
            };
            crate::setup::hook_eval_for_target(target, stdin, stdout, stderr, self_check)
        }
        [flag, ..]
            if ["--stats", "--discover", "--err", "--test"]
                .iter()
                .any(|deferred| flag == OsStr::new(deferred)) =>
        {
            stderr.write_all(b"usage: tapas does not expose deferred state modes in 0.2.0\n")?;
            Ok(2)
        }
        [flag, ..] if is_setup_flag(flag) => {
            let Some(request) = setup_request(args) else {
                stderr.write_all(SETUP_USAGE)?;
                return Ok(2);
            };
            if request.force
                && (request.action != crate::setup::Action::Setup
                    || request.target != crate::setup::Target::OpenCode)
            {
                stderr.write_all(SETUP_USAGE)?;
                return Ok(2);
            }
            crate::setup::configure_for_target_with_force(
                request.action,
                request.target,
                request.dry_run,
                request.force,
                stdout,
                stderr,
            )
        }
        [flag, ..] if flag.as_encoded_bytes().starts_with(b"-") => {
            stderr.write_all(b"usage: tapas [--help|--version|--filters] <cmd...>\n")?;
            Ok(2)
        }
        command @ [_, ..] => crate::process::run(
            command,
            stdout,
            stderr,
            crate::process::RunOptions::default(),
        )
        .map(|report| report.exit_code),
    }
}

fn stdin_is_tty() -> bool {
    // SAFETY: isatty only inspects the process's valid standard-input file descriptor.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn should_wrap(command: &[OsString]) -> bool {
    let Some(program) = command.first() else {
        return false;
    };
    let basename = catalog::command_basename(program);
    basename != Some(OsStr::new("tapas")) && catalog::should_auto_wrap(program)
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

fn is_setup_flag(argument: &OsStr) -> bool {
    let bytes = argument.as_encoded_bytes();
    bytes == b"--setup"
        || bytes == b"--unsetup"
        || bytes.starts_with(b"--setup=")
        || bytes.starts_with(b"--unsetup=")
}

fn hook_request(args: &[OsString]) -> Option<(crate::setup::Target, bool)> {
    match args {
        [flag, target] if flag == OsStr::new("--hook-eval") => {
            Some((crate::setup::Target::parse(target)?, false))
        }
        [flag, target, self_check]
            if flag == OsStr::new("--hook-eval") && self_check == OsStr::new("--self-check") =>
        {
            Some((crate::setup::Target::parse(target)?, true))
        }
        _ => None,
    }
}

struct SetupRequest {
    action: crate::setup::Action,
    target: crate::setup::Target,
    dry_run: bool,
    force: bool,
}

fn setup_request(args: &[OsString]) -> Option<SetupRequest> {
    let first = args.first()?.as_encoded_bytes();
    let (flag, target, option_start) =
        if let Some(separator) = first.iter().position(|byte| *byte == b'=') {
            (&first[..separator], &first[separator + 1..], 1)
        } else {
            (first, args.get(1)?.as_encoded_bytes(), 2)
        };
    let action = match flag {
        b"--setup" => crate::setup::Action::Setup,
        b"--unsetup" => crate::setup::Action::Unsetup,
        _ => return None,
    };
    let mut request = SetupRequest {
        action,
        target: crate::setup::Target::parse_bytes(target)?,
        dry_run: false,
        force: false,
    };
    for option in &args[option_start..] {
        match option.as_encoded_bytes() {
            b"--dry-run" if !request.dry_run => request.dry_run = true,
            b"--force" if !request.force => request.force = true,
            _ => return None,
        }
    }
    Some(request)
}
