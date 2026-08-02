use super::*;

pub(super) fn matches_jest(input: &[u8]) -> bool {
    find_subslice(input, b"Test Suites:").is_some()
        || find_subslice(input, b"Test Files").is_some()
        || input.starts_with(b"Tests:")
        || find_subslice(input, b"\nTests:").is_some()
}

pub(super) fn apply_jest(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_jest(stdout, &mut output);
    scan_jest(stderr, &mut output);
    if output.is_empty() || !has_jest_failure(&output) {
        b"all tests passed\n".to_vec()
    } else {
        head_tail(output, 120, 80)
    }
}

fn scan_jest(input: &[u8], output: &mut Vec<u8>) {
    let mut in_error_context = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii();
        if line.is_empty() {
            in_error_context = false;
            continue;
        }
        if line.starts_with(b"PASS ") || line.starts_with(b"PASS\t") {
            in_error_context = false;
            continue;
        }
        if line.starts_with("✓ ".as_bytes())
            || line.starts_with(b"Snapshots:")
            || line.starts_with(b"Time:")
            || line.starts_with(b"Ran all")
        {
            continue;
        }
        if should_keep_jest(line) {
            in_error_context = true;
            append_line(output, line);
        } else if in_error_context {
            append_line(output, line);
        }
    }
}

fn should_keep_jest(line: &[u8]) -> bool {
    [
        b"FAIL ".as_slice(),
        b"FAIL\t",
        "● ".as_bytes(),
        b"Expected:",
        b"Received:",
        b"expect(",
        b"Error:",
        b"    at ",
        b"Test Suites:",
        b"Tests:",
        b"Test Files",
        "✗ ".as_bytes(),
        "✕ ".as_bytes(),
        b" failed",
        b"Failed",
        b"    > ",
        b">   ",
        b"E   ",
    ]
    .iter()
    .any(|needle| find_subslice(line, needle).is_some())
}

fn has_jest_failure(input: &[u8]) -> bool {
    if find_subslice(input, b"FAIL").is_some() || find_subslice(input, "●".as_bytes()).is_some() {
        return true;
    }
    input
        .split(|byte| *byte == b'\n')
        .any(|line| nonzero_count_before(line, b"failed"))
}

pub(super) fn nonzero_count_before(line: &[u8], marker: &[u8]) -> bool {
    let Some(end) = find_subslice(line, marker) else {
        return false;
    };
    if end < 2 || line[end - 1] != b' ' {
        return false;
    }
    let mut start = end - 1;
    while start > 0 && line[start - 1].is_ascii_digit() {
        start -= 1;
    }
    start < end - 1 && line[start..end - 1].iter().any(|byte| *byte != b'0')
}
