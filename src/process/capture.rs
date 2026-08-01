use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use super::unix;

pub(super) const DRAIN_GRACE: Duration = Duration::from_millis(500);
pub(super) const READ_BUFFER_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy)]
pub enum CaptureMode {
    Buffered { limit: usize },
    Passthrough,
}

pub struct CapturedOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
    pub incomplete: bool,
    pub streamed: bool,
    pub overflowed: bool,
    pub input_bytes: usize,
}

pub fn run_inherited(argv: &[OsString]) -> io::Result<i32> {
    let mut command = command(argv)?;
    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    Ok(unix::exit_code(status))
}

pub fn run_captured(
    argv: &[OsString],
    mode: CaptureMode,
    force_c_locale: bool,
    raw_stdout: &mut dyn Write,
    raw_stderr: &mut dyn Write,
) -> io::Result<CapturedOutput> {
    let mut command = command(argv)?;
    if force_c_locale {
        command.env("LC_ALL", "C").env("LANG", "C");
    }
    let mut child = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let result = drain_child(&mut child, mode, raw_stdout, raw_stderr);
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

pub(super) fn command(argv: &[OsString]) -> io::Result<Command> {
    let Some(program) = argv.first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing child command",
        ));
    };
    let mut command = Command::new(program);
    command.args(&argv[1..]);
    Ok(command)
}

fn drain_child(
    child: &mut Child,
    mode: CaptureMode,
    raw_stdout: &mut dyn Write,
    raw_stderr: &mut dyn Write,
) -> io::Result<CapturedOutput> {
    let mut child_stdout = child.stdout.take().expect("piped child stdout");
    let mut child_stderr = child.stderr.take().expect("piped child stderr");
    unix::set_nonblocking(child_stdout.as_raw_fd())?;
    unix::set_nonblocking(child_stderr.as_raw_fd())?;

    let mut state = DrainState::new(mode, raw_stdout, raw_stderr);
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut stdout_buffer = [0_u8; READ_BUFFER_BYTES];
    let mut stderr_buffer = [0_u8; READ_BUFFER_BYTES];
    let mut child_status: Option<ExitStatus> = None;
    let mut exit_observed_at: Option<Instant> = None;
    let mut incomplete = false;

    while stdout_open || stderr_open {
        unix::poll_readable(
            stdout_open.then(|| child_stdout.as_raw_fd()),
            stderr_open.then(|| child_stderr.as_raw_fd()),
        )?;

        if stdout_open {
            stdout_open = read_available(&mut child_stdout, &mut stdout_buffer, |chunk| {
                state.accept(Stream::Stdout, chunk)
            })?;
        }
        if stderr_open {
            stderr_open = read_available(&mut child_stderr, &mut stderr_buffer, |chunk| {
                state.accept(Stream::Stderr, chunk)
            })?;
        }

        if child_status.is_none()
            && let Some(status) = child.try_wait()?
        {
            child_status = Some(status);
            exit_observed_at = Some(Instant::now());
        }
        if exit_observed_at.is_some_and(|observed| observed.elapsed() >= DRAIN_GRACE) {
            incomplete = stdout_open || stderr_open;
            break;
        }
    }

    drop(child_stdout);
    drop(child_stderr);
    let status = match child_status {
        Some(status) => status,
        None => child.wait()?,
    };
    Ok(state.finish(unix::exit_code(status), incomplete))
}

pub(super) fn read_available(
    reader: &mut (impl Read + ?Sized),
    buffer: &mut [u8; READ_BUFFER_BYTES],
    mut accept: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<bool> {
    for _ in 0..2 {
        match reader.read(buffer) {
            Ok(0) => return Ok(false),
            Ok(read) => accept(&buffer[..read])?,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

struct DrainState<'a> {
    mode: DrainMode,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    raw_stdout: &'a mut dyn Write,
    raw_stderr: &'a mut dyn Write,
    input_bytes: usize,
}

#[derive(Clone, Copy)]
enum DrainMode {
    Buffered { limit: usize },
    Passthrough,
    Overflowed,
}

impl<'a> DrainState<'a> {
    fn new(
        mode: CaptureMode,
        raw_stdout: &'a mut dyn Write,
        raw_stderr: &'a mut dyn Write,
    ) -> Self {
        let mode = match mode {
            CaptureMode::Buffered { limit } => DrainMode::Buffered { limit },
            CaptureMode::Passthrough => DrainMode::Passthrough,
        };
        Self {
            mode,
            stdout: Vec::new(),
            stderr: Vec::new(),
            raw_stdout,
            raw_stderr,
            input_bytes: 0,
        }
    }

    fn accept(&mut self, stream: Stream, chunk: &[u8]) -> io::Result<()> {
        self.input_bytes += chunk.len();
        if !matches!(self.mode, DrainMode::Buffered { .. }) {
            return self.write_raw(stream, chunk);
        }

        match stream {
            Stream::Stdout => self.stdout.extend_from_slice(chunk),
            Stream::Stderr => self.stderr.extend_from_slice(chunk),
        }
        let limit = match self.mode {
            DrainMode::Buffered { limit } => limit,
            DrainMode::Passthrough | DrainMode::Overflowed => return self.write_raw(stream, chunk),
        };
        if self.stdout.len() >= limit || self.stderr.len() >= limit {
            self.mode = DrainMode::Overflowed;
            self.raw_stdout.write_all(&self.stdout)?;
            self.raw_stderr.write_all(&self.stderr)?;
            self.stdout.clear();
            self.stderr.clear();
        }
        Ok(())
    }

    fn write_raw(&mut self, stream: Stream, chunk: &[u8]) -> io::Result<()> {
        match stream {
            Stream::Stdout => self.raw_stdout.write_all(chunk),
            Stream::Stderr => self.raw_stderr.write_all(chunk),
        }
    }

    fn finish(self, exit_code: i32, incomplete: bool) -> CapturedOutput {
        let streamed = !matches!(self.mode, DrainMode::Buffered { .. });
        let overflowed = matches!(self.mode, DrainMode::Overflowed);
        CapturedOutput {
            stdout: self.stdout,
            stderr: self.stderr,
            exit_code,
            incomplete,
            streamed,
            overflowed,
            input_bytes: self.input_bytes,
        }
    }
}
