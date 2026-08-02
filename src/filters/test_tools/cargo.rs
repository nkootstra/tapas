pub(super) fn matches_cargo_test(input: &[u8]) -> bool {
    if find_subslice(input, b"test result:").is_some() {
        return true;
    }
    input.split(|byte| *byte == b'\n').any(|line| {
        let line = line.trim_ascii_start();
        line.starts_with(b"running ") && find_subslice(line, b" test").is_some()
    })
}

pub(super) fn apply_cargo_test(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_cargo_test(stdout, &mut output);
    scan_cargo_test(stderr, &mut output);
    if output.is_empty() {
        b"all tests passed\n".to_vec()
    } else {
        head_tail(output, 120, 80)
    }
}

fn scan_cargo_test(input: &[u8], output: &mut Vec<u8>) {
    let mut before = VecDeque::<Vec<u8>>::with_capacity(3);
    let mut in_error_context = false;
    let mut after_remaining = 0;
    let mut dropping_names = false;

    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii();
        if line.is_empty() {
            in_error_context = false;
            after_remaining = 0;
            dropping_names = false;
            continue;
        }
        if line.starts_with(b"note: run with") {
            continue;
        }
        if line == b"failures:" {
            dropping_names = true;
            continue;
        }
        if dropping_names {
            if raw.starts_with(b"    ") || raw.starts_with(b"\t") {
                continue;
            }
            dropping_names = false;
        }

        if should_keep_cargo(line) {
            for context in before.drain(..) {
                append_line(output, &context);
            }
            in_error_context = line.starts_with(b"error") || line.starts_with(b"warning");
            after_remaining = 3;
            if line.starts_with(b"test result:") {
                write_cargo_result(output, line);
                in_error_context = false;
                after_remaining = 0;
            } else {
                append_line(output, line);
            }
            continue;
        }
        if in_error_context && is_cargo_error_context(line) {
            append_line(output, line);
            continue;
        }
        if after_remaining > 0 {
            append_line(output, line);
            after_remaining -= 1;
            continue;
        }
        in_error_context = false;
        if before.len() == 3 {
            before.pop_front();
        }
        before.push_back(line.to_vec());
    }
}

fn should_keep_cargo(line: &[u8]) -> bool {
    if line.starts_with(b"error: test failed, to rerun pass") {
        return false;
    }
    [
        b"error[".as_slice(),
        b"error:",
        b"warning:",
        b"test result:",
        b"panicked at",
        b"---- ",
        b"bench:",
    ]
    .iter()
    .any(|needle| find_subslice(line, needle).is_some())
}

fn is_cargo_error_context(line: &[u8]) -> bool {
    line.starts_with(b"-->")
        || line.starts_with(b"= ")
        || line.first().is_some_and(u8::is_ascii_digit)
        || line.starts_with(b"|")
        || line.starts_with(b"^")
        || line.starts_with(b"-")
        || line.starts_with(b"For more info")
}

fn write_cargo_result(output: &mut Vec<u8>, line: &[u8]) {
    output.extend_from_slice(b"res ");
    output.extend_from_slice(number_before(line, b" passed").unwrap_or(b"0"));
    output.extend_from_slice(b"p ");
    output.extend_from_slice(number_before(line, b" failed").unwrap_or(b"0"));
    output.push(b'f');
    if let Some(position) = find_subslice(line, b"finished in ") {
        let duration = first_token(&line[position + b"finished in ".len()..]);
        if !duration.is_empty() {
            output.push(b' ');
            output.extend_from_slice(duration);
        }
    }
    output.push(b'\n');
}

fn number_before<'a>(line: &'a [u8], marker: &[u8]) -> Option<&'a [u8]> {
    let end = find_subslice(line, marker)?;
    let mut start = end;
    while start > 0 && line[start - 1].is_ascii_digit() {
        start -= 1;
    }
    (start < end).then_some(&line[start..end])
}

fn first_token(input: &[u8]) -> &[u8] {
    let input = input.trim_ascii();
    let end = input
        .iter()
        .position(|byte| matches!(byte, b' ' | b'\t'))
        .unwrap_or(input.len());
    &input[..end]
}

pub(super) fn head_tail(input: Vec<u8>, head: usize, tail: usize) -> Vec<u8> {
    let lines: Vec<&[u8]> = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() <= head + tail {
        return input;
    }
    let omitted = lines.len() - head - tail;
    let mut output = Vec::with_capacity(input.len());
    for line in &lines[..head] {
        append_line(&mut output, line);
    }
    output.extend_from_slice(b"(tapas: omitted ");
    output.extend_from_slice(omitted.to_string().as_bytes());
    output.extend_from_slice(b" relevant lines; rerun with tapas --raw)\n");
    for line in &lines[lines.len() - tail..] {
        append_line(&mut output, line);
    }
    output
}
use super::{append_line, find_subslice, strip_ansi};
use std::collections::VecDeque;
