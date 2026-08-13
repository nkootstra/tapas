pub(super) struct LogState {
    compose: bool,
    preserve_metadata: bool,
    raw_passthrough: bool,
    pending: Vec<u8>,
    fingerprint: Vec<u8>,
    repeats: usize,
}

impl LogState {
    pub(super) fn new(compose: bool, preserve_metadata: bool) -> Self {
        Self {
            compose,
            preserve_metadata,
            raw_passthrough: false,
            pending: Vec::new(),
            fingerprint: Vec::new(),
            repeats: 0,
        }
    }

    pub(super) fn feed_line(&mut self, line: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        if self.raw_passthrough || self.preserve_metadata {
            writer.write_all(line)?;
            return writer.write_all(b"\n");
        }
        let clean = strip_ansi(line);
        let classified = clean.trim_ascii_end();
        if classified.is_empty() {
            return self.flush(writer);
        }
        if self.compose && !is_compose_line(classified) {
            self.flush(writer)?;
            writer.write_all(line)?;
            writer.write_all(b"\n")?;
            self.raw_passthrough = true;
            return Ok(());
        }
        if self.compose && !classified.contains(&b'|') {
            self.flush(writer)?;
            writer.write_all(classified)?;
            return writer.write_all(b"\n");
        }
        let normalized = normalize_log_line(classified, self.compose);
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

fn is_compose_line(line: &[u8]) -> bool {
    if line.contains(&b'|') {
        return line[..line.iter().position(|byte| *byte == b'|').unwrap_or(0)]
            .trim_ascii()
            .iter()
            .any(|byte| !byte.is_ascii_whitespace());
    }
    let line = line.trim_ascii_start();
    line.starts_with(b"[+]")
        || line.starts_with(b"#")
        || [b"Container ".as_slice(), b"Network ", b"Volume "]
            .iter()
            .any(|kind| line.starts_with(kind) || contains_word(line, kind))
        || [
            b" Built".as_slice(),
            b" Created",
            b" Recreated",
            b" Started",
            b" Healthy",
        ]
        .iter()
        .any(|state| line.ends_with(state))
}

fn contains_word(line: &[u8], word: &[u8]) -> bool {
    line.windows(word.len() + 1)
        .any(|window| window[0].is_ascii_whitespace() && &window[1..] == word)
}
use super::{normalize_log_line, strip_ansi, timestamp_end};
use std::io::{self, Write};
