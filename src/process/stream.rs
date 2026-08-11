use std::ffi::OsString;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use super::capture::{self, DRAIN_GRACE, READ_BUFFER_BYTES};
use super::unix;
use crate::filters::{
    find_subslice, normalize_log_line, rfind_subslice, strip_ansi_csi as strip_ansi, test_tools,
    timestamp_end,
};

const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_FRAME_BYTES: usize = 512 * 1024;
const IDLE_FLUSH: Duration = Duration::from_secs(2);
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

pub(super) struct StreamedOutput {
    pub exit_code: i32,
    pub input_bytes: usize,
    pub displayed_bytes: usize,
    pub changed: bool,
    pub incomplete: bool,
    pub filter_name: &'static str,
}

pub(super) fn run(
    argv: &[OsString],
    logical_argv: &[OsString],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<StreamedOutput> {
    let kind = StreamKind::classify(logical_argv);
    let mut command = capture::command(argv)?;
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (mut child, forwarder) = unix::spawn_process_group(&mut command)?;
    let result = (|| {
        let mut child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("child stdout pipe was not created"))?;
        let mut child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("child stderr pipe was not created"))?;
        unix::set_nonblocking(child_stdout.as_raw_fd())?;
        unix::set_nonblocking(child_stderr.as_raw_fd())?;

        let mut stdout_count = CountingWriter::new(stdout);
        let mut stderr_count = CountingWriter::new(stderr);
        let mut stdout_side = StreamSide::new(kind);
        let mut stderr_side = StreamSide::new(kind);
        let mut stdout_open = true;
        let mut stderr_open = true;
        let mut stdout_buffer = [0_u8; READ_BUFFER_BYTES];
        let mut stderr_buffer = [0_u8; READ_BUFFER_BYTES];
        let mut child_status: Option<ExitStatus> = None;
        let mut exit_observed_at: Option<Instant> = None;
        let mut last_activity = Instant::now();
        let mut input_bytes = 0usize;
        let mut input_stdout_fingerprint = ByteFingerprint::new();
        let mut input_stderr_fingerprint = ByteFingerprint::new();
        let mut incomplete = false;

        while stdout_open || stderr_open {
            forwarder.forward_pending()?;
            unix::poll_readable(
                stdout_open.then(|| child_stdout.as_raw_fd()),
                stderr_open.then(|| child_stderr.as_raw_fd()),
            )?;
            forwarder.forward_pending()?;
            let mut received = false;
            if stdout_open {
                stdout_open =
                    capture::read_available(&mut child_stdout, &mut stdout_buffer, |chunk| {
                        received = true;
                        input_bytes += chunk.len();
                        input_stdout_fingerprint.absorb(chunk);
                        stdout_side.feed(chunk, &mut stdout_count)
                    })?;
            }
            if stderr_open {
                stderr_open =
                    capture::read_available(&mut child_stderr, &mut stderr_buffer, |chunk| {
                        received = true;
                        input_bytes += chunk.len();
                        input_stderr_fingerprint.absorb(chunk);
                        stderr_side.feed(chunk, &mut stderr_count)
                    })?;
            }
            if received {
                stdout_count.flush()?;
                stderr_count.flush()?;
                last_activity = Instant::now();
            } else if last_activity.elapsed() >= IDLE_FLUSH {
                stdout_side.idle_flush(&mut stdout_count)?;
                stderr_side.idle_flush(&mut stderr_count)?;
                stdout_count.flush()?;
                stderr_count.flush()?;
                last_activity = Instant::now();
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
        stdout_side.finish(&mut stdout_count)?;
        stderr_side.finish(&mut stderr_count)?;
        stdout_count.flush()?;
        stderr_count.flush()?;
        let displayed_bytes = stdout_count.count + stderr_count.count;
        let status = match child_status {
            Some(status) => status,
            None => unix::wait_for_child(&mut child, &forwarder)?,
        };
        let changed = stdout_count.changed_from(&input_stdout_fingerprint)
            || stderr_count.changed_from(&input_stderr_fingerprint);
        Ok(StreamedOutput {
            exit_code: unix::exit_code(status),
            input_bytes,
            displayed_bytes,
            changed,
            incomplete,
            filter_name: kind.filter_name(),
        })
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[derive(Clone, Copy)]
enum StreamKind {
    Logs { compose: bool, docker: bool },
    Tsc,
    Jest,
    Gh,
}

impl StreamKind {
    fn classify(argv: &[OsString]) -> Self {
        let command = argv
            .first()
            .and_then(|value| crate::catalog::command_basename(value.as_os_str()))
            .map_or(b"".as_slice(), |value| value.as_encoded_bytes());
        if command == b"tsc" {
            return Self::Tsc;
        }
        if matches!(command, b"jest" | b"vitest") {
            return Self::Jest;
        }
        if command == b"gh" {
            return Self::Gh;
        }
        let compose = command == b"docker-compose"
            || command == b"docker" && argv.get(1).is_some_and(|value| value == "compose");
        Self::Logs {
            compose,
            docker: matches!(command, b"docker" | b"docker-compose"),
        }
    }

    fn filter_name(self) -> &'static str {
        match self {
            Self::Logs { docker: true, .. } => "stream:docker_logs",
            Self::Logs { .. } => "stream:logs",
            Self::Tsc => "stream:tsc_watch",
            Self::Jest => "stream:js_test_watch",
            Self::Gh => "stream:gh_run_watch",
        }
    }
}

struct CountingWriter<'a> {
    inner: &'a mut dyn Write,
    count: usize,
    fingerprint: ByteFingerprint,
}

impl<'a> CountingWriter<'a> {
    fn new(inner: &'a mut dyn Write) -> Self {
        Self {
            inner,
            count: 0,
            fingerprint: ByteFingerprint::new(),
        }
    }

    fn changed_from(&self, input: &ByteFingerprint) -> bool {
        self.fingerprint != *input
    }
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        let written_bytes = &bytes[..written];
        self.fingerprint.absorb(written_bytes);
        self.count += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct ByteFingerprint {
    bytes: usize,
    hash: u64,
}

impl ByteFingerprint {
    fn new() -> Self {
        Self {
            bytes: 0,
            hash: FNV_OFFSET_BASIS,
        }
    }

    fn absorb(&mut self, input: &[u8]) {
        self.bytes += input.len();
        for byte in input {
            self.hash ^= *byte as u64;
            self.hash = self.hash.wrapping_mul(FNV_PRIME);
        }
    }
}

struct StreamSide {
    line: Vec<u8>,
    processor: Processor,
}

impl StreamSide {
    fn new(kind: StreamKind) -> Self {
        let processor = match kind {
            StreamKind::Logs { compose, .. } => Processor::Logs(LogState::new(compose)),
            StreamKind::Tsc => Processor::Tsc(TscState::default()),
            StreamKind::Jest => Processor::Jest(JestState::default()),
            StreamKind::Gh => Processor::Gh(GhState::default()),
        };
        Self {
            line: Vec::new(),
            processor,
        }
    }

    fn feed(&mut self, bytes: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        let mut start = 0usize;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte != b'\n' {
                continue;
            }
            self.line.extend_from_slice(&bytes[start..index]);
            self.emit_line(writer)?;
            start = index + 1;
        }
        if start < bytes.len() {
            self.line.extend_from_slice(&bytes[start..]);
            if self.line.len() >= MAX_LINE_BYTES {
                self.emit_line(writer)?;
            }
        }
        Ok(())
    }

    fn emit_line(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        let line = std::mem::take(&mut self.line);
        let result = self.processor.feed_line(&line, writer);
        self.line = line;
        self.line.clear();
        result
    }

    fn idle_flush(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        self.processor.idle_flush(writer)
    }

    fn finish(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        if !self.line.is_empty() {
            self.emit_line(writer)?;
        }
        self.processor.finish(writer)
    }
}

enum Processor {
    Logs(LogState),
    Tsc(TscState),
    Jest(JestState),
    Gh(GhState),
}

impl Processor {
    fn feed_line(&mut self, line: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        match self {
            Self::Logs(state) => state.feed_line(line, writer),
            Self::Tsc(state) => state.feed_line(line, writer),
            Self::Jest(state) => state.feed_line(line, writer),
            Self::Gh(state) => state.feed_line(line, writer),
        }
    }

    fn idle_flush(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        match self {
            Self::Logs(state) => state.flush(writer),
            Self::Jest(state) => state.flush(writer),
            Self::Gh(state) => state.flush_pending(writer),
            Self::Tsc(_) => Ok(()),
        }
    }

    fn finish(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        match self {
            Self::Logs(state) => state.flush(writer),
            Self::Jest(state) => state.flush(writer),
            Self::Gh(state) => state.finish(writer),
            Self::Tsc(_) => Ok(()),
        }
    }
}

mod github;
mod jest;
mod line;
mod logs;
mod tsc;

use github::GhState;
use jest::JestState;
use logs::LogState;
use tsc::TscState;

#[cfg(test)]
mod tests {
    use super::{MAX_FRAME_BYTES, MAX_LINE_BYTES, StreamKind, StreamSide};

    #[test]
    fn log_lines_are_assembled_bounded_and_deduplicated() {
        let mut side = StreamSide::new(StreamKind::Logs {
            compose: false,
            docker: true,
        });
        let mut output = Vec::new();
        side.feed(b"2026-08-01 10:00:00 INFO rea", &mut output)
            .unwrap();
        side.feed(b"dy\n2026-08-01 10:00:01 INFO ready\nnext\n", &mut output)
            .unwrap();
        side.finish(&mut output).unwrap();
        assert_eq!(output, "INFO ready ×2\nnext\n".as_bytes());

        let mut side = StreamSide::new(StreamKind::Logs {
            compose: false,
            docker: false,
        });
        let mut output = Vec::new();
        side.feed(&vec![b'x'; MAX_LINE_BYTES], &mut output).unwrap();
        side.finish(&mut output).unwrap();
        assert_eq!(output.len(), MAX_LINE_BYTES + 1);
    }

    #[test]
    fn tsc_watch_emits_diagnostics_and_clean_transitions() {
        let mut side = StreamSide::new(StreamKind::Tsc);
        let mut output = Vec::new();
        side.feed(
            concat!(
                "[10:00:00] Starting compilation in watch mode...\n",
                "src/app.ts:1:7 - error TS2322: bad type\n",
                "        ~\n",
                "Found 1 error. Watching for file changes.\n",
                "Found 0 errors. Watching for file changes.\n",
                "Found 0 errors. Watching for file changes.\n",
            )
            .as_bytes(),
            &mut output,
        )
        .unwrap();
        side.finish(&mut output).unwrap();
        assert_eq!(
            output,
            concat!(
                "src/app.ts:1:7 TS2322: bad type\n",
                "Found 1 error. Watching for file changes.\n",
                "clean (0 errors)\n",
            )
            .as_bytes()
        );
    }

    #[test]
    fn jest_frames_and_gh_jobs_only_emit_transitions() {
        let frame = concat!(
            "FAIL src/app.test.ts\n",
            "  ● works\n",
            "    Expected: 1\n",
            "    Received: 2\n",
            "Test Suites: 1 failed, 1 total\n",
            "Tests:       1 failed, 1 total\n",
        );
        let mut jest = StreamSide::new(StreamKind::Jest);
        let mut output = Vec::new();
        jest.feed(frame.as_bytes(), &mut output).unwrap();
        jest.idle_flush(&mut output).unwrap();
        jest.feed(b"\x1b[2J\x1b[H", &mut output).unwrap();
        jest.feed(frame.as_bytes(), &mut output).unwrap();
        jest.finish(&mut output).unwrap();
        assert_eq!(
            output
                .windows(b"Test Suites:".len())
                .filter(|window| *window == b"Test Suites:")
                .count(),
            1
        );

        let mut gh = StreamSide::new(StreamKind::Gh);
        let mut output = Vec::new();
        gh.feed(
            concat!(
                "JOBS\n",
                "* build (ID 123)\n",
                "  * checkout\n",
                "\n",
                "JOBS\n",
                "✓ build in 2s (ID 123)\n",
                "\n",
            )
            .as_bytes(),
            &mut output,
        )
        .unwrap();
        gh.finish(&mut output).unwrap();
        assert_eq!(output, b"build: running\nbuild: running->passed\n");
    }

    #[test]
    fn watch_frames_fail_open_before_buffers_can_grow_unbounded() {
        let mut gh = StreamSide::new(StreamKind::Gh);
        let mut compact = Vec::new();
        gh.feed(b"\x1b[31mwaiting\x1b[0m   \n", &mut compact)
            .unwrap();
        gh.finish(&mut compact).unwrap();
        assert_eq!(compact, b"waiting\n");

        let line = vec![b'x'; MAX_LINE_BYTES - 1];
        let mut input = Vec::new();
        while input.len() <= MAX_FRAME_BYTES {
            input.extend_from_slice(&line);
            input.push(b'\n');
        }

        for kind in [StreamKind::Jest, StreamKind::Gh] {
            let mut side = StreamSide::new(kind);
            let mut output = Vec::new();
            side.feed(&input, &mut output).unwrap();
            side.finish(&mut output).unwrap();
            assert_eq!(output, input);
        }
    }
}
