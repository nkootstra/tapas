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
  tapas --setup claude [--dry-run]
  tapas --unsetup claude [--dry-run]

Options:
  -h, --help       Show this help
  --version        Show the Tapas version
  --filters        List the static compatibility catalogs
";
const INTERNAL_BOUNDARY_STATUS: i32 = 70;
const SETUP_USAGE: &[u8] =
    b"usage: tapas --setup claude [--dry-run]\n       tapas --unsetup claude [--dry-run]\n";

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
        [] if environment_flag_on("TAPAS_LOSSLESS") => {
            io::copy(stdin, stdout)?;
            Ok(0)
        }
        [] => {
            stderr
                .write_all(b"tapas: stdin filtering is not available in the foundation build\n")?;
            Ok(INTERNAL_BOUNDARY_STATUS)
        }
        [arg] if arg == OsStr::new("--version") => {
            writeln!(stdout, "tapas {}", env!("CARGO_PKG_VERSION"))?;
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
            process_boundary(stderr)
        }
        [flag, command @ ..] if flag == OsStr::new("--explain") && !command.is_empty() => {
            process_boundary(stderr)
        }
        [flag, command @ ..] if flag == OsStr::new("--rewrite") && !command.is_empty() => {
            if should_wrap(command) {
                stdout.write_all(b"tapas ")?;
            }
            write_shell_command(stdout, command)?;
            stdout.write_all(b"\n")?;
            Ok(0)
        }
        [flag, target] if flag == OsStr::new("--hook-eval") && target == OsStr::new("claude") => {
            stderr.write_all(
                b"tapas: Claude hook evaluation is not available in the foundation build\n",
            )?;
            Ok(INTERNAL_BOUNDARY_STATUS)
        }
        [flag, ..] if flag == OsStr::new("--hook-eval") => {
            stderr.write_all(b"usage: tapas --hook-eval claude\n")?;
            Ok(2)
        }
        [flag, ..]
            if ["--stats", "--discover", "--err", "--test"]
                .iter()
                .any(|deferred| flag == OsStr::new(deferred)) =>
        {
            stderr.write_all(b"usage: tapas does not expose deferred state modes in 0.1.0\n")?;
            Ok(2)
        }
        args if is_claude_setup(args) => {
            stderr.write_all(b"tapas: Claude setup is not available in the foundation build\n")?;
            Ok(INTERNAL_BOUNDARY_STATUS)
        }
        [flag, ..] if is_setup_flag(flag) => {
            stderr.write_all(SETUP_USAGE)?;
            Ok(2)
        }
        [flag, ..] if flag.as_encoded_bytes().starts_with(b"-") => {
            stderr.write_all(b"usage: tapas [--help|--version|--filters] <cmd...>\n")?;
            Ok(2)
        }
        [_, ..] => process_boundary(stderr),
    }
}

fn process_boundary(stderr: &mut dyn Write) -> io::Result<i32> {
    if environment_flag_on("TAPAS_STREAM") {
        stderr.write_all(
            b"tapas: streaming process execution is not available in the foundation build\n",
        )?;
    } else {
        stderr.write_all(b"tapas: process execution is not available in the foundation build\n")?;
    }
    Ok(INTERNAL_BOUNDARY_STATUS)
}

fn stdin_is_tty() -> bool {
    // SAFETY: isatty only inspects the process's valid standard-input file descriptor.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn environment_flag_on(name: &str) -> bool {
    std::env::var_os(name).and_then(|value| value.as_encoded_bytes().first().copied()) == Some(b'1')
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

fn is_claude_setup(args: &[OsString]) -> bool {
    matches!(
        args,
        [flag, target]
            if (flag == OsStr::new("--setup") || flag == OsStr::new("--unsetup"))
                && target == OsStr::new("claude")
    ) || matches!(
        args,
        [flag, target, dry_run]
            if (flag == OsStr::new("--setup") || flag == OsStr::new("--unsetup"))
                && target == OsStr::new("claude")
                && dry_run == OsStr::new("--dry-run")
    ) || matches!(
        args,
        [flag]
            if flag == OsStr::new("--setup=claude")
                || flag == OsStr::new("--unsetup=claude")
    ) || matches!(
        args,
        [flag, dry_run]
            if (flag == OsStr::new("--setup=claude")
                || flag == OsStr::new("--unsetup=claude"))
                && dry_run == OsStr::new("--dry-run")
    )
}
