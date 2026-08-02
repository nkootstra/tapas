use super::*;

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
pub(super) struct GhState {
    raw_fallback: Vec<u8>,
    rendered_fallback: Vec<u8>,
    pending_name: Vec<u8>,
    pending_status: Option<JobStatus>,
    pending_has_steps: bool,
    states: Vec<(Vec<u8>, JobStatus)>,
    saw_jobs: bool,
    in_jobs: bool,
    raw_passthrough: bool,
}

impl GhState {
    pub(super) fn feed_line(&mut self, raw: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        if self.raw_passthrough {
            return append_written_line(writer, raw);
        }
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        let trimmed = line.trim_ascii();
        if trimmed == b"JOBS" {
            self.flush_pending(writer)?;
            self.saw_jobs = true;
            self.in_jobs = true;
            self.raw_fallback.clear();
            self.rendered_fallback.clear();
            return Ok(());
        }
        if !self.saw_jobs {
            if self.raw_fallback.len().saturating_add(raw.len() + 1) > MAX_FRAME_BYTES {
                writer.write_all(&self.raw_fallback)?;
                append_written_line(writer, raw)?;
                self.raw_fallback.clear();
                self.rendered_fallback.clear();
                self.raw_passthrough = true;
                return Ok(());
            }
            self.raw_fallback.extend_from_slice(raw);
            self.raw_fallback.push(b'\n');
            if !line.is_empty() || !self.rendered_fallback.is_empty() {
                self.rendered_fallback.extend_from_slice(line);
                self.rendered_fallback.push(b'\n');
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

    pub(super) fn flush_pending(&mut self, writer: &mut dyn Write) -> io::Result<()> {
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

    pub(super) fn finish(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        self.flush_pending(writer)?;
        if !self.saw_jobs {
            writer.write_all(&self.rendered_fallback)?;
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
