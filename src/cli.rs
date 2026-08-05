use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};

use crate::catalog;

pub(crate) mod spec;

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
            spec::write_help(stdout)?;
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
        _ => run_arguments(args, stdin, stdout, stderr),
    }
}

fn run_arguments(
    args: &[OsString],
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<i32> {
    match spec::Mode::parse(&args[0]) {
        Some(spec::Mode::Version) if args.len() == 1 => {
            writeln!(stdout, "tapas {}", env!("TAPAS_BUILD_LABEL"))?;
            Ok(0)
        }
        Some(spec::Mode::Version) => usage_error(stderr, b"--version does not accept arguments"),
        Some(spec::Mode::Help) if args.len() == 1 => {
            spec::write_help(stdout)?;
            Ok(0)
        }
        Some(spec::Mode::Help) => usage_error(stderr, b"--help does not accept arguments"),
        Some(spec::Mode::Filters) if args.len() == 1 => {
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
        Some(spec::Mode::Filters) => usage_error(stderr, b"--filters does not accept arguments"),
        Some(spec::Mode::Completions) => {
            let [_, shell] = args else {
                return usage_error(stderr, b"--completions requires bash, zsh, or fish");
            };
            let Some(shell) = spec::Shell::parse(shell) else {
                return usage_error(stderr, b"--completions requires bash, zsh, or fish");
            };
            crate::completions::write(shell, stdout)?;
            Ok(0)
        }
        Some(spec::Mode::Raw) => {
            let rest = &args[1..];
            if rest.is_empty() {
                if stdin_is_tty() {
                    return usage_error(stderr, b"--raw requires a command or piped input");
                }
                io::copy(stdin, stdout)?;
                return Ok(0);
            }
            let command = rest.strip_prefix(&[OsString::from("--")]).unwrap_or(rest);
            if command.is_empty() {
                return usage_error(stderr, b"--raw requires a command after --");
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
        Some(spec::Mode::Explain) if args.len() > 1 => crate::process::run(
            &args[1..],
            stdout,
            stderr,
            crate::process::RunOptions {
                raw: false,
                explain: true,
            },
        )
        .map(|report| report.exit_code),
        Some(spec::Mode::Explain) => usage_error(stderr, b"--explain requires a command"),
        Some(spec::Mode::Rewrite) if args.len() > 1 => {
            let command = &args[1..];
            if should_wrap(command) {
                stdout.write_all(b"tapas ")?;
            }
            write_shell_command(stdout, command)?;
            stdout.write_all(b"\n")?;
            Ok(0)
        }
        Some(spec::Mode::Rewrite) => usage_error(stderr, b"--rewrite requires a command"),
        Some(spec::Mode::HookEval) => {
            let Some((target, self_check)) = hook_request(&args[1..]) else {
                return usage_error(stderr, b"--hook-eval requires claude, codex, or opencode");
            };
            crate::setup::hook_eval_for_target(target, stdin, stdout, stderr, self_check)
        }
        Some(mode @ (spec::Mode::Setup | spec::Mode::Unsetup)) => {
            let Some(request) = setup_request(mode, args) else {
                return usage_error(stderr, b"invalid --setup or --unsetup arguments");
            };
            if request.force
                && (request.action != crate::setup::Action::Setup
                    || request.target != crate::setup::Target::OpenCode)
            {
                return usage_error(stderr, b"invalid --setup or --unsetup arguments");
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
        None if is_deferred_mode(&args[0]) => {
            let flag = &args[0];
            stderr.write_all(b"tapas: option ")?;
            write!(stderr, "{flag:?}")?;
            stderr.write_all(b" is not available in Tapas 0.2.0\n\n")?;
            spec::write_help(stderr)?;
            Ok(2)
        }
        None if args[0].as_encoded_bytes().starts_with(b"-") => {
            stderr.write_all(b"tapas: unknown option ")?;
            write!(stderr, "{:?}", args[0])?;
            stderr.write_all(b"\n\n")?;
            spec::write_help(stderr)?;
            Ok(2)
        }
        None => crate::process::run(args, stdout, stderr, crate::process::RunOptions::default())
            .map(|report| report.exit_code),
    }
}

fn usage_error(stderr: &mut dyn Write, explanation: &[u8]) -> io::Result<i32> {
    stderr.write_all(b"tapas: ")?;
    stderr.write_all(explanation)?;
    stderr.write_all(b"\n\n")?;
    spec::write_help(stderr)?;
    Ok(2)
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

fn is_deferred_mode(argument: &OsStr) -> bool {
    ["--stats", "--discover", "--err", "--test"]
        .iter()
        .any(|deferred| argument == OsStr::new(deferred))
}

fn hook_request(args: &[OsString]) -> Option<(crate::setup::Target, bool)> {
    match args {
        [target] => Some((crate::setup::Target::parse(target)?, false)),
        [target, self_check] if self_check == OsStr::new("--self-check") => {
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

fn setup_request(mode: spec::Mode, args: &[OsString]) -> Option<SetupRequest> {
    let first = args.first()?.as_encoded_bytes();
    let (target, option_start) =
        if let Some(separator) = first.iter().position(|byte| *byte == b'=') {
            (&first[separator + 1..], 1)
        } else {
            (args.get(1)?.as_encoded_bytes(), 2)
        };
    let action = match mode {
        spec::Mode::Setup => crate::setup::Action::Setup,
        spec::Mode::Unsetup => crate::setup::Action::Unsetup,
        _ => unreachable!("setup_request only handles setup modes"),
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
