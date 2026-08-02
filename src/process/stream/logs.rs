pub(super) struct LogState {
    compose: bool,
    pending: Vec<u8>,
    fingerprint: Vec<u8>,
    repeats: usize,
}

impl LogState {
    pub(super) fn new(compose: bool) -> Self {
        Self {
            compose,
            pending: Vec::new(),
            fingerprint: Vec::new(),
            repeats: 0,
        }
    }

    pub(super) fn feed_line(&mut self, line: &[u8], writer: &mut dyn Write) -> io::Result<()> {
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

    pub(super) fn flush(&mut self, writer: &mut dyn Write) -> io::Result<()> {
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
use super::{normalize_log_line, strip_ansi, timestamp_end};
use std::io::{self, Write};
