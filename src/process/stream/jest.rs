use super::*;

#[derive(Default)]
pub(super) struct JestState {
    frame: Vec<u8>,
    last_emitted: Vec<u8>,
    raw_passthrough: bool,
}

impl JestState {
    pub(super) fn feed_line(&mut self, raw: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        if self.raw_passthrough {
            return append_written_line(writer, raw);
        }
        let line = if let Some(index) = clear_frame_index(raw) {
            self.flush(writer)?;
            if self.raw_passthrough {
                return append_written_line(writer, raw);
            }
            &raw[index..]
        } else {
            raw
        };
        if self.frame.len().saturating_add(line.len() + 1) > MAX_FRAME_BYTES {
            writer.write_all(&self.frame)?;
            append_written_line(writer, line)?;
            self.frame.clear();
            self.last_emitted.clear();
            self.raw_passthrough = true;
            return Ok(());
        }
        self.frame.extend_from_slice(line);
        self.frame.push(b'\n');
        Ok(())
    }

    pub(super) fn flush(&mut self, writer: &mut dyn Write) -> io::Result<()> {
        if self.frame.is_empty() {
            return Ok(());
        }
        let frame = std::mem::take(&mut self.frame);
        match test_tools::apply_matched(&frame) {
            Ok(output) if !output.bytes.is_empty() && output.bytes != self.last_emitted => {
                writer.write_all(&output.bytes)?;
                self.last_emitted = output.bytes;
            }
            Ok(_) => {}
            Err(_) => {
                writer.write_all(&frame)?;
                self.last_emitted.clear();
                self.raw_passthrough = true;
            }
        }
        self.frame = frame;
        self.frame.clear();
        Ok(())
    }
}
