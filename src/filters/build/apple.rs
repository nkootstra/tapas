use super::*;

pub(super) fn compact_apple_build(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for input in [stdout, stderr] {
        scan_apple_build(input, &mut output);
    }
    if output.is_empty() && !stdout.is_empty() && stderr.is_empty() {
        b"ok\n".to_vec()
    } else {
        output
    }
}

fn scan_apple_build(input: &[u8], output: &mut Vec<u8>) {
    if input.is_empty() {
        return;
    }
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = trim_ascii_end(&clean);
        let trimmed = trim_ascii_start(line);
        if should_keep_apple_build(trimmed) {
            append_line(output, line);
        }
    }
}

fn should_keep_apple_build(line: &[u8]) -> bool {
    contains_ignore_ascii_case(line, b"error:")
        || contains_ignore_ascii_case(line, b"warning:")
        || [
            b"** BUILD FAILED **".as_slice(),
            b"** BUILD SUCCEEDED **",
            b"** TEST FAILED **",
            b"** TEST SUCCEEDED **",
            b"SwiftCompile",
            b"CompileSwift",
            b"Failing tests:",
            b"Test Suite",
            b"Executed ",
        ]
        .iter()
        .any(|needle| find_subslice(line, needle).is_some())
}

pub(super) fn compact_package_tool(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for input in [stdout, stderr] {
        scan_package_tool(input, &mut output);
    }
    if output.is_empty() && !stdout.is_empty() && stderr.is_empty() {
        b"ok\n".to_vec()
    } else {
        output
    }
}

fn scan_package_tool(input: &[u8], output: &mut Vec<u8>) {
    if input.is_empty() {
        return;
    }
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = trim_ascii_end(&clean);
        if should_keep_package_tool(trim_ascii_start(line)) {
            append_line(output, line);
        }
    }
}

fn should_keep_package_tool(line: &[u8]) -> bool {
    if line.starts_with(b"Preparing packages") {
        return false;
    }
    let package_delta = matches!(line.first(), Some(b'+' | b'-'))
        && find_subslice(line.get(1..).unwrap_or_default(), b"==").is_some();
    package_delta
        || find_subslice(line, b"ERR!").is_some()
        || find_subslice(line, b"WARN").is_some()
        || [
            b"error".as_slice(),
            b"failed",
            b"deprecated",
            b"vulnerab",
            b"added ",
            b"removed ",
            b"changed ",
            b"packages",
            b"done in",
        ]
        .iter()
        .any(|needle| contains_ignore_ascii_case(line, needle))
        || line.starts_with(b"\xe2\x9c\x93")
        || line.starts_with(b"\xe2\x9c\x95")
}

pub(super) fn matches_gradle(input: &[u8]) -> bool {
    [
        b"BUILD FAILED".as_slice(),
        b"BUILD SUCCESSFUL",
        b"FAILURE: Build failed",
        b"> Task ",
        b" tests completed",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}
