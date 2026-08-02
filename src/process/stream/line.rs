use super::*;

pub(super) fn clear_frame_index(line: &[u8]) -> Option<usize> {
    match (
        find_subslice(line, b"\x1b[2J"),
        find_subslice(line, b"\x1b[H"),
    ) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
}

pub(super) fn append_written_line(writer: &mut dyn Write, line: &[u8]) -> io::Result<()> {
    writer.write_all(line)?;
    writer.write_all(b"\n")
}
