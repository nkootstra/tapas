#[derive(Default)]
pub(super) struct TscState {
    clean_emitted: bool,
    diagnostic_context: bool,
    raw: bool,
}

impl TscState {
    pub(super) fn feed_line(&mut self, line: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        if self.raw {
            return append_written_line(writer, line);
        }
        let clean = strip_ansi(line);
        let trimmed = clean.trim_ascii();
        if trimmed.is_empty() {
            if self.diagnostic_context {
                return append_written_line(writer, line);
            }
            return Ok(());
        }
        if trimmed.starts_with(b"Found 0 errors") {
            if !self.clean_emitted {
                writer.write_all(b"clean (0 errors)\n")?;
                self.clean_emitted = true;
            }
            self.diagnostic_context = false;
            return Ok(());
        }
        if trimmed.starts_with(b"Found ") && find_subslice(trimmed, b"error").is_some() {
            append_written_line(writer, trimmed)?;
            self.clean_emitted = false;
            self.diagnostic_context = false;
            return Ok(());
        }
        if let Some(index) = find_subslice(trimmed, b" - error TS") {
            let rest = &trimmed[index + b" - error ".len()..];
            if rest.contains(&b':') {
                writer.write_all(&trimmed[..index])?;
                writer.write_all(b" ")?;
                append_written_line(writer, rest)?;
                self.clean_emitted = false;
                self.diagnostic_context = true;
                return Ok(());
            }
        }
        if find_subslice(trimmed, b"error TS").is_some() {
            append_written_line(writer, trimmed)?;
            self.clean_emitted = false;
            self.diagnostic_context = true;
            return Ok(());
        }
        if self.diagnostic_context {
            return append_written_line(writer, line);
        }
        if find_subslice(trimmed, b"Starting compilation in watch mode").is_some()
            || find_subslice(
                trimmed,
                b"File change detected. Starting incremental compilation",
            )
            .is_some()
        {
            return Ok(());
        }
        self.raw = true;
        append_written_line(writer, line)
    }
}
use super::line::append_written_line;
use super::{find_subslice, strip_ansi};
use std::io::{self, Write};
