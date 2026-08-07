use super::super::{append_line, find_subslice, strip_ansi_csi as strip_ansi};

pub(super) fn matches_lint(input: &[u8]) -> bool {
    find_subslice(input, b" problems").is_some()
        || find_subslice(input, b" problem").is_some()
        || find_subslice(input, b"lint/").is_some()
        || find_subslice(input, b"error").is_some() && find_subslice(input, b"warning").is_some()
}

pub(super) fn compact_lint(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut state = LintState::default();
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_lint(stdout, &mut output, &mut state);
    scan_lint(stderr, &mut output, &mut state);
    if !state.emitted && (!stdout.is_empty() || !stderr.is_empty()) {
        output.extend_from_slice(b"lint ok\n");
    }
    output
}

#[derive(Default)]
struct LintState {
    pending_file: Vec<u8>,
    pending_emitted: bool,
    emitted: bool,
}

fn scan_lint(input: &[u8], output: &mut Vec<u8>, state: &mut LintState) {
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if looks_like_file_header(line) {
            state.pending_file.clear();
            state.pending_file.extend_from_slice(line);
            state.pending_emitted = false;
        } else if looks_like_lint_diagnostic(line) {
            if !state.pending_file.is_empty() && !state.pending_emitted {
                append_line(output, &state.pending_file);
                state.pending_emitted = true;
            }
            append_collapsed(output, line);
            state.emitted = true;
        } else if looks_like_lint_summary(line) {
            append_line(output, line);
            state.emitted = true;
        }
    }
}

fn looks_like_file_header(line: &[u8]) -> bool {
    !line.is_empty()
        && !line.contains(&b' ')
        && (line.contains(&b'/')
            || [
                b".js".as_slice(),
                b".jsx",
                b".ts",
                b".tsx",
                b".vue",
                b".svelte",
            ]
            .iter()
            .any(|suffix| line.ends_with(suffix)))
}

fn looks_like_lint_diagnostic(line: &[u8]) -> bool {
    if ![b"error".as_slice(), b"warning", b"lint/"]
        .iter()
        .any(|needle| find_subslice(line, needle).is_some())
    {
        return false;
    }
    if numeric_location_prefix(line) {
        return true;
    }
    let Some(first) = line.iter().position(|byte| *byte == b':') else {
        return false;
    };
    numeric_location_prefix(&line[first + 1..])
}

fn numeric_location_prefix(line: &[u8]) -> bool {
    let mut cursor = 0;
    while line.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == 0 || line.get(cursor) != Some(&b':') {
        return false;
    }
    cursor += 1;
    let start = cursor;
    while line.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    cursor > start && cursor < line.len()
}

fn looks_like_lint_summary(line: &[u8]) -> bool {
    find_subslice(line, b" problems").is_some()
        || find_subslice(line, b" problem").is_some()
        || line.starts_with(b"Found ")
        || line.starts_with(b"Checked ")
        || line.starts_with(b"No lint errors")
}

fn append_collapsed(output: &mut Vec<u8>, line: &[u8]) {
    let mut previous_space = false;
    for byte in line {
        if *byte == b' ' {
            if !previous_space {
                output.push(b' ');
            }
            previous_space = true;
        } else {
            output.push(*byte);
            previous_space = false;
        }
    }
    output.push(b'\n');
}
