use super::super::{append_line, find_subslice, strip_ansi_csi as strip_ansi};

pub(super) fn compact_ruff(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut current_path = Vec::new();
    scan_ruff(stdout, &mut output, &mut current_path);
    scan_ruff(stderr, &mut output, &mut current_path);
    output
}

fn scan_ruff(input: &[u8], output: &mut Vec<u8>, current_path: &mut Vec<u8>) {
    for raw in input.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if let Some((path, location, body)) = parse_ruff_diagnostic(line) {
            if current_path != path {
                append_line(output, path);
                current_path.clear();
                current_path.extend_from_slice(path);
            }
            output.extend_from_slice(b"  ");
            output.extend_from_slice(location);
            output.push(b' ');
            append_line(output, body);
        } else if is_ruff_summary(line) {
            append_line(output, line);
            current_path.clear();
        }
    }
}

fn parse_ruff_diagnostic(line: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    let first = line.iter().position(|byte| *byte == b':')?;
    let mut cursor = first + 1;
    let line_start = cursor;
    while line.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == line_start || line.get(cursor) != Some(&b':') {
        return None;
    }
    cursor += 1;
    let column_start = cursor;
    while line.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == column_start || line.get(cursor) != Some(&b':') {
        return None;
    }
    Some((
        &line[..first],
        &line[first + 1..cursor],
        line[cursor + 1..].trim_ascii_start(),
    ))
}

fn is_ruff_summary(line: &[u8]) -> bool {
    line.starts_with(b"All checks passed")
        || line.starts_with(b"Found ")
        || line.ends_with(b"would be reformatted")
        || line.ends_with(b"left unchanged")
        || find_subslice(line, b" files would be reformatted").is_some()
        || find_subslice(line, b" files left unchanged").is_some()
}
