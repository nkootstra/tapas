use std::ffi::OsString;
use std::io::{self, Read, Write};

mod execute;
mod invocation;
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
    if args.is_empty() {
        return run_pipe(stdin, stdout);
    }

    execute::run(invocation::parse(args), stdin, stdout, stderr)
}

fn run_pipe(stdin: &mut dyn Read, stdout: &mut dyn Write) -> io::Result<i32> {
    if stdin_is_tty() {
        spec::write_help(stdout)?;
    } else if crate::environment::flag_on("TAPAS_LOSSLESS") {
        io::copy(stdin, stdout)?;
    } else {
        crate::pipeline::run(stdin, stdout)?;
    }
    Ok(0)
}

fn stdin_is_tty() -> bool {
    // SAFETY: isatty only inspects the process's valid standard-input file descriptor.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}
