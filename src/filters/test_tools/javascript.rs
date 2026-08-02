use super::*;

#[derive(Clone, Copy)]
enum JsTestMode {
    Mocha,
    Node,
}

pub(super) fn matches_js_test(input: &[u8]) -> bool {
    matches_mocha(input) || matches_node_test(input)
}

fn matches_mocha(input: &[u8]) -> bool {
    input.split(|byte| *byte == b'\n').any(|line| {
        let line = line.trim_ascii();
        is_number_summary(line, b" passing") || is_number_summary(line, b" failing")
    })
}

fn matches_node_test(input: &[u8]) -> bool {
    let mut has_tests = false;
    let mut has_result = false;
    for line in input.split(|byte| *byte == b'\n') {
        let line = line.trim_ascii();
        has_tests |= line.starts_with(b"# tests ");
        has_result |= line.starts_with(b"# fail ")
            || line.starts_with(b"# pass ")
            || line.starts_with(b"not ok ")
            || starts_with_unicode_failure(line);
    }
    has_tests && has_result
}

pub(super) fn apply_js_test(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mode = if matches_node_test(stdout) || matches_node_test(stderr) {
        JsTestMode::Node
    } else {
        JsTestMode::Mocha
    };
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    match mode {
        JsTestMode::Mocha => {
            scan_mocha(stdout, &mut output);
            scan_mocha(stderr, &mut output);
        }
        JsTestMode::Node => {
            scan_node_test(stdout, &mut output);
            scan_node_test(stderr, &mut output);
        }
    }
    if output.is_empty() {
        b"all tests passed\n".to_vec()
    } else {
        output
    }
}

fn scan_mocha(input: &[u8], output: &mut Vec<u8>) {
    let input = input.strip_suffix(b"\n").unwrap_or(input);
    if input.is_empty() {
        return;
    }
    let mut in_failure = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii_end();
        let trimmed = line.trim_ascii();
        if starts_with_unicode_pass(trimmed) {
            continue;
        }
        if is_number_summary(trimmed, b" passing") || is_number_summary(trimmed, b" failing") {
            in_failure = false;
            append_line(output, trimmed);
        } else if is_mocha_failure_header(trimmed) {
            in_failure = true;
            append_line(output, trimmed);
        } else if in_failure {
            append_line(output, line);
        }
    }
}

fn scan_node_test(input: &[u8], output: &mut Vec<u8>) {
    let input = input.strip_suffix(b"\n").unwrap_or(input);
    if input.is_empty() {
        return;
    }
    let mut in_failure = false;
    let mut skipping_pass = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii_end();
        let trimmed = line.trim_ascii();
        if skipping_pass {
            if trimmed == b"..." {
                skipping_pass = false;
            }
            continue;
        }
        if starts_with_unicode_pass(trimmed) {
            continue;
        }
        if trimmed.starts_with(b"ok ") {
            skipping_pass = true;
            continue;
        }
        if trimmed.starts_with(b"TAP version ") {
            continue;
        }
        if is_node_trailer(trimmed) {
            in_failure = false;
            append_line(output, trimmed);
        } else if trimmed.starts_with(b"not ok ") || starts_with_unicode_failure(trimmed) {
            in_failure = true;
            append_line(output, trimmed);
        } else if in_failure {
            append_line(output, line);
            if trimmed == b"..." {
                in_failure = false;
            }
        }
    }
}

fn is_number_summary(line: &[u8], marker: &[u8]) -> bool {
    let Some(position) = find_subslice(line, marker) else {
        return false;
    };
    if position == 0 || !line[..position].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let after = &line[position + marker.len()..];
    after.is_empty() || (after.len() >= 2 && after[0] == b' ' && after[1] == b'(')
}

fn is_mocha_failure_header(line: &[u8]) -> bool {
    let digits = line
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(line.len());
    digits > 0 && line.get(digits..digits + 2) == Some(b") ")
}

fn starts_with_unicode_pass(line: &[u8]) -> bool {
    line.starts_with("✔ ".as_bytes()) || line.starts_with("✓ ".as_bytes())
}

fn starts_with_unicode_failure(line: &[u8]) -> bool {
    line.starts_with("✖ ".as_bytes()) || line.starts_with("✕ ".as_bytes())
}

fn is_node_trailer(line: &[u8]) -> bool {
    line.starts_with(b"1..")
        || [
            b"# tests ".as_slice(),
            b"# suites ",
            b"# pass ",
            b"# fail ",
            b"# cancelled ",
            b"# skipped ",
            b"# todo ",
            b"# duration_ms ",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}
