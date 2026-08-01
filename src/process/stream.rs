use std::ffi::OsString;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use super::capture::{self, DRAIN_GRACE, READ_BUFFER_BYTES};
use super::unix;
use crate::filters::test_tools;

const MAX_LINE_BYTES: usize = 64 * 1024;
const IDLE_FLUSH: Duration = Duration::from_secs(2);

pub(super) struct StreamedOutput {
    pub exit_code: i32,
    pub input_bytes: usize,
    pub displayed_bytes: usize,
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
    let mut child = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let result = (|| {
        let mut child_stdout = child.stdout.take().expect("piped child stdout");
        let mut child_stderr = child.stderr.take().expect("piped child stderr");
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
        let mut incomplete = false;

        while stdout_open || stderr_open {
            unix::poll_readable(
                stdout_open.then(|| child_stdout.as_raw_fd()),
                stderr_open.then(|| child_stderr.as_raw_fd()),
            )?;
            let mut received = false;
            if stdout_open {
                stdout_open =
                    capture::read_available(&mut child_stdout, &mut stdout_buffer, |chunk| {
                        received = true;
                        input_bytes += chunk.len();
                        stdout_side.feed(chunk, &mut stdout_count)
                    })?;
            }
            if stderr_open {
                stderr_open =
                    capture::read_available(&mut child_stderr, &mut stderr_buffer, |chunk| {
                        received = true;
                        input_bytes += chunk.len();
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
            None => child.wait()?,
        };
        Ok(StreamedOutput {
            exit_code: unix::exit_code(status),
            input_bytes,
            displayed_bytes,
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
}

impl<'a> CountingWriter<'a> {
    fn new(inner: &'a mut dyn Write) -> Self {
        Self { inner, count: 0 }
    }
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.count += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
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

struct LogState {
    compose: bool,
    pending: Vec<u8>,
    fingerprint: Vec<u8>,
    repeats: usize,
}

impl LogState {
    fn new(compose: bool) -> Self {
        Self {
            compose,
            pending: Vec::new(),
            fingerprint: Vec::new(),
            repeats: 0,
        }
    }

    fn feed_line(&mut self, line: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        let clean = strip_ansi(line);
        let line = clean.trim_ascii_end();
        if line.is_empty() {
            return self.flush(writer);
        }
        let normalized = normalize_log_line(line, self.compose);
        let start = if self.compose {
            0
        } else {
            timestamp_end(&normalized)
        };
        if self.repeats > 0 && normalized[start..] == self.fingerprint {
            self.repeats += 1;
            return Ok(());
        }
        self.flush(writer)?;
        self.fingerprint.extend_from_slice(&normalized[start..]);
        self.pending = normalized;
        self.repeats = 1;
        Ok(())
    }

    fn flush(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        if self.repeats == 0 {
            return Ok(());
        }
        let start = timestamp_end(&self.pending);
        writer.write_all(if start > 0 {
            &self.pending[start..]
        } else {
            &self.pending
        })?;
        if self.repeats > 1 {
            write!(writer, " ×{}", self.repeats)?;
        }
        writer.write_all(b"\n")?;
        self.pending.clear();
        self.fingerprint.clear();
        self.repeats = 0;
        Ok(())
    }
}

#[derive(Default)]
struct TscState {
    clean_emitted: bool,
}

impl TscState {
    fn feed_line(&mut self, line: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        let clean = strip_ansi(line);
        let trimmed = clean.trim_ascii();
        if trimmed.is_empty() {
            return Ok(());
        }
        if trimmed.starts_with(b"Found 0 errors") {
            if !self.clean_emitted {
                writer.write_all(b"clean (0 errors)\n")?;
                self.clean_emitted = true;
            }
            return Ok(());
        }
        if trimmed.starts_with(b"Found ") && find_subslice(trimmed, b"error").is_some() {
            append_written_line(writer, trimmed)?;
            self.clean_emitted = false;
            return Ok(());
        }
        if let Some(index) = find_subslice(trimmed, b" - error TS") {
            let rest = &trimmed[index + b" - error ".len()..];
            if rest.contains(&b':') {
                writer.write_all(&trimmed[..index])?;
                writer.write_all(b" ")?;
                append_written_line(writer, rest)?;
                self.clean_emitted = false;
                return Ok(());
            }
        }
        if find_subslice(trimmed, b"error TS").is_some() {
            append_written_line(writer, trimmed)?;
            self.clean_emitted = false;
        }
        Ok(())
    }
}

#[derive(Default)]
struct JestState {
    frame: Vec<u8>,
    last_emitted: Vec<u8>,
}

impl JestState {
    fn feed_line(&mut self, raw: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        let line = if let Some(index) = clear_frame_index(raw) {
            self.flush(writer)?;
            &raw[index..]
        } else {
            raw
        };
        self.frame.extend_from_slice(line);
        self.frame.push(b'\n');
        Ok(())
    }

    fn flush(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        if self.frame.is_empty() {
            return Ok(());
        }
        let frame = std::mem::take(&mut self.frame);
        let rendered = if test_tools::matches(&frame) {
            test_tools::apply_matched(&frame)
                .ok()
                .map(|output| output.bytes)
        } else {
            None
        };
        self.frame = frame;
        self.frame.clear();
        if let Some(rendered) = rendered
            && !rendered.is_empty()
            && rendered != self.last_emitted
        {
            writer.write_all(&rendered)?;
            self.last_emitted = rendered;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum JobStatus {
    Queued,
    Running,
    Passed,
    Failed,
    Skipped,
}

impl JobStatus {
    fn label(self) -> &'static [u8] {
        match self {
            Self::Queued => b"queued",
            Self::Running => b"running",
            Self::Passed => b"passed",
            Self::Failed => b"failed",
            Self::Skipped => b"skipped",
        }
    }
}

#[derive(Default)]
struct GhState {
    raw_fallback: Vec<u8>,
    pending_name: Vec<u8>,
    pending_status: Option<JobStatus>,
    pending_has_steps: bool,
    states: Vec<(Vec<u8>, JobStatus)>,
    saw_jobs: bool,
    in_jobs: bool,
}

impl GhState {
    fn feed_line(&mut self, raw: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        let trimmed = line.trim_ascii();
        if trimmed == b"JOBS" {
            self.flush_pending(writer)?;
            self.saw_jobs = true;
            self.in_jobs = true;
            self.raw_fallback.clear();
            return Ok(());
        }
        if !self.saw_jobs {
            if !line.is_empty() || !self.raw_fallback.is_empty() {
                self.raw_fallback.extend_from_slice(line);
                self.raw_fallback.push(b'\n');
            }
            return Ok(());
        }
        if !self.in_jobs {
            return Ok(());
        }
        if trimmed.is_empty() || matches!(trimmed, b"ANNOTATIONS" | b"ARTIFACTS") {
            self.flush_pending(writer)?;
            self.in_jobs = false;
            return Ok(());
        }
        if let Some((name, status)) = parse_gh_job(line) {
            self.flush_pending(writer)?;
            self.pending_name.extend_from_slice(name);
            self.pending_status = Some(status);
            self.pending_has_steps = false;
            return Ok(());
        }
        if gh_status_prefix(line).is_some_and(|(_, _, step)| step) {
            self.pending_has_steps = true;
            return Ok(());
        }
        self.flush_pending(writer)?;
        self.in_jobs = false;
        Ok(())
    }

    fn flush_pending(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        let Some(pending) = self.pending_status.take() else {
            return Ok(());
        };
        let status = if pending == JobStatus::Queued && self.pending_has_steps {
            JobStatus::Running
        } else {
            pending
        };
        self.pending_has_steps = false;
        let name = std::mem::take(&mut self.pending_name);
        if let Some((_, previous)) = self.states.iter_mut().find(|(known, _)| *known == name) {
            if *previous != status {
                writer.write_all(&name)?;
                writer.write_all(b": ")?;
                writer.write_all(previous.label())?;
                writer.write_all(b"->")?;
                writer.write_all(status.label())?;
                writer.write_all(b"\n")?;
                *previous = status;
            }
            self.pending_name = name;
            self.pending_name.clear();
            return Ok(());
        }
        writer.write_all(&name)?;
        writer.write_all(b": ")?;
        writer.write_all(status.label())?;
        writer.write_all(b"\n")?;
        self.states.push((name, status));
        Ok(())
    }

    fn finish(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        self.flush_pending(writer)?;
        if !self.saw_jobs {
            writer.write_all(&self.raw_fallback)?;
        }
        Ok(())
    }
}

fn parse_gh_job(line: &[u8]) -> Option<(&[u8], JobStatus)> {
    let (prefix, status, step) = gh_status_prefix(line)?;
    if step {
        return None;
    }
    let rest = line[prefix..].trim_ascii_end();
    let without_close = rest.strip_suffix(b")")?;
    let marker = rfind_subslice(without_close, b" (ID ")?;
    let id = &without_close[marker + b" (ID ".len()..];
    if id.is_empty() || !id.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut name = rest[..marker].trim_ascii();
    if let Some(index) = rfind_subslice(name, b" in ")
        && looks_like_duration(&name[index + b" in ".len()..])
    {
        name = name[..index].trim_ascii_end();
    }
    (!name.is_empty()).then_some((name, status))
}

fn gh_status_prefix(line: &[u8]) -> Option<(usize, JobStatus, bool)> {
    let step = line.starts_with(b"  ");
    let offset = usize::from(step) * 2;
    let rest = line.get(offset..)?;
    for (prefix, status) in [
        ("✓ ".as_bytes(), JobStatus::Passed),
        (b"X ".as_slice(), JobStatus::Failed),
        (b"- ", JobStatus::Skipped),
        (b"* ", JobStatus::Queued),
    ] {
        if rest.starts_with(prefix) {
            return Some((offset + prefix.len(), status, step));
        }
    }
    None
}

fn looks_like_duration(input: &[u8]) -> bool {
    !input.is_empty()
        && !input.contains(&b' ')
        && input.iter().any(u8::is_ascii_digit)
        && input
            .iter()
            .any(|byte| matches!(byte, b'h' | b'm' | b's' | b'.' | b'u'))
        && input
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'h' | b'm' | b's' | b'.' | b'u'))
}

fn normalize_log_line(line: &[u8], compose: bool) -> Vec<u8> {
    if compose && let Some(pipe) = line.iter().position(|byte| *byte == b'|') {
        let service = line[..pipe].trim_ascii();
        if !service.is_empty() {
            let payload = line[pipe + 1..].trim_ascii_start();
            let payload = &payload[timestamp_end(payload)..];
            let mut normalized = service.to_vec();
            normalized.push(b'|');
            if !payload.is_empty() {
                normalized.push(b' ');
                normalized.extend_from_slice(payload);
            }
            return normalized;
        }
    }
    line.to_vec()
}

fn timestamp_end(line: &[u8]) -> usize {
    if line.len() < 10
        || !line[..4].iter().all(u8::is_ascii_digit)
        || line[4] != b'-'
        || !line[5..7].iter().all(u8::is_ascii_digit)
        || line[7] != b'-'
        || !line[8..10].iter().all(u8::is_ascii_digit)
    {
        return 0;
    }
    let mut cursor = if line.get(10) == Some(&b' ')
        && line.len() >= 19
        && line[11..13].iter().all(u8::is_ascii_digit)
        && line[13] == b':'
        && line[14..16].iter().all(u8::is_ascii_digit)
        && line[16] == b':'
        && line[17..19].iter().all(u8::is_ascii_digit)
    {
        19
    } else {
        0
    };
    while cursor < line.len() && !matches!(line[cursor], b' ' | b'\t') {
        cursor += 1;
    }
    while line
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    cursor
}

fn clear_frame_index(line: &[u8]) -> Option<usize> {
    match (
        find_subslice(line, b"\x1b[2J"),
        find_subslice(line, b"\x1b[H"),
    ) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
}

fn strip_ansi(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0usize;
    while index < input.len() {
        if input[index] == 0x1b && input.get(index + 1) == Some(&b'[') {
            let mut end = index + 2;
            while end < input.len() {
                let byte = input[end];
                end += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
            if end <= input.len() {
                index = end;
                continue;
            }
        }
        output.push(input[index]);
        index += 1;
    }
    output
}

fn append_written_line(writer: &mut dyn Write, line: &[u8]) -> io::Result<()> {
    writer.write_all(line)?;
    writer.write_all(b"\n")
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (!needle.is_empty() && needle.len() <= haystack.len())
        .then(|| {
            haystack
                .windows(needle.len())
                .position(|window| window == needle)
        })
        .flatten()
}

fn rfind_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (!needle.is_empty() && needle.len() <= haystack.len())
        .then(|| {
            haystack
                .windows(needle.len())
                .rposition(|window| window == needle)
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::{MAX_LINE_BYTES, StreamKind, StreamSide};

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
}
