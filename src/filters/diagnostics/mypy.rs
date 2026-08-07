use super::super::{append_line, find_subslice, strip_ansi_csi as strip_ansi};

pub(super) fn compact_mypy(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_mypy(stdout, &mut output);
    scan_mypy(stderr, &mut output);
    output
}

fn scan_mypy(input: &[u8], output: &mut Vec<u8>) {
    let mut in_diagnostic = false;
    for raw in input.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            in_diagnostic = false;
            continue;
        }
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if is_mypy_diagnostic(line)
            || line.starts_with(b"Found ")
            || line.starts_with(b"Success: ")
            || line.starts_with(b"mypy: ")
        {
            append_line(output, line);
            in_diagnostic = is_mypy_diagnostic(line);
        } else if in_diagnostic && is_caret_line(line) {
            append_line(output, line);
            in_diagnostic = false;
        }
    }
}

fn is_mypy_diagnostic(line: &[u8]) -> bool {
    find_subslice(line, b": error:").is_some() || find_subslice(line, b": note:").is_some()
}

fn is_caret_line(line: &[u8]) -> bool {
    let line = line.trim_ascii();
    !line.is_empty() && line.iter().all(|byte| matches!(byte, b'^' | b'~'))
}
