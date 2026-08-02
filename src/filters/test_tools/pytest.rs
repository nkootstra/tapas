use super::*;

pub(super) fn matches_pytest(input: &[u8]) -> bool {
    if find_subslice(input, b"test session starts").is_some()
        || find_subslice(input, b"passed in ").is_some()
        || find_subslice(input, b"failed in ").is_some()
    {
        return true;
    }
    if find_subslice(input, b"collected ").is_some() {
        return input.split(|byte| *byte == b'\n').any(|line| {
            find_subslice(line, b"collected ").is_some() && find_subslice(line, b" item").is_some()
        });
    }
    false
}

pub(super) fn apply_pytest(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_pytest(stdout, &mut output);
    scan_pytest(stderr, &mut output);
    if output.is_empty() || !has_pytest_failure(&output) {
        b"all tests passed\n".to_vec()
    } else {
        head_tail(output, 120, 80)
    }
}

fn scan_pytest(input: &[u8], output: &mut Vec<u8>) {
    let mut in_error_context = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii();
        if line.is_empty() {
            in_error_context = false;
            continue;
        }
        if should_keep_pytest(line) {
            in_error_context = true;
            let without_frame = trim_bytes(line, |byte| matches!(byte, b'=' | b' '));
            append_line(
                output,
                if without_frame.is_empty() {
                    line
                } else {
                    without_frame
                },
            );
        } else if in_error_context {
            append_line(output, line);
        }
    }
}

fn should_keep_pytest(line: &[u8]) -> bool {
    [
        b"FAILED".as_slice(),
        b"ERROR",
        b"failed",
        b"error",
        b"assert",
        b"collected",
        b"short test summary",
        b"==== ",
        b">   ",
        b"E   ",
    ]
    .iter()
    .any(|needle| find_subslice(line, needle).is_some())
}

fn has_pytest_failure(input: &[u8]) -> bool {
    find_subslice(input, b"FAILED").is_some()
        || find_subslice(input, b"ERROR").is_some()
        || input
            .split(|byte| *byte == b'\n')
            .any(|line| nonzero_count_before(line, b"failed"))
}

fn trim_bytes(input: &[u8], predicate: impl Fn(u8) -> bool) -> &[u8] {
    let start = input
        .iter()
        .position(|byte| !predicate(*byte))
        .unwrap_or(input.len());
    let end = input
        .iter()
        .rposition(|byte| !predicate(*byte))
        .map_or(start, |position| position + 1);
    &input[start..end]
}
