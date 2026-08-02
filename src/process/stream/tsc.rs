#[derive(Default)]
pub(super) struct TscState {
    clean_emitted: bool,
}

impl TscState {
    pub(super) fn feed_line(&mut self, line: &[u8], writer: &mut dyn Write) -> io::Result<()> {
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
use super::line::append_written_line;
use super::{find_subslice, strip_ansi};
use std::io::{self, Write};
