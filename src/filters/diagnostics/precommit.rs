use super::super::{append_line, strip_ansi_csi as strip_ansi};

pub(super) fn matches_precommit(input: &[u8]) -> bool {
    input
        .split(|byte| *byte == b'\n')
        .any(|raw| hook_status(strip_ansi(raw).trim_ascii_end()).is_some())
}

pub(super) fn compact_precommit(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut passed = 0usize;
    scan_precommit(stdout, &mut output, &mut passed);
    scan_precommit(stderr, &mut output, &mut passed);
    if passed > 0 {
        output.extend_from_slice(b"passed: ");
        output.extend_from_slice(passed.to_string().as_bytes());
        output.extend_from_slice(if passed == 1 { b" hook\n" } else { b" hooks\n" });
    }
    output
}

fn scan_precommit(input: &[u8], output: &mut Vec<u8>, passed: &mut usize) {
    let mut in_failure = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if let Some(status) = hook_status(line) {
            in_failure = status == b"Failed";
            if status == b"Passed" {
                *passed += 1;
            } else if status != b"Skipped" {
                append_depadded_status(output, line, status);
            }
        } else if in_failure {
            append_line(output, line);
        }
    }
}

fn hook_status(line: &[u8]) -> Option<&'static [u8]> {
    [b"Passed".as_slice(), b"Failed", b"Skipped"]
        .into_iter()
        .find(|status| line_ends_with_dot_status(line, status))
}

fn line_ends_with_dot_status(line: &[u8], status: &[u8]) -> bool {
    let Some(prefix) = line.strip_suffix(status) else {
        return false;
    };
    prefix
        .iter()
        .rev()
        .take_while(|byte| **byte == b'.')
        .count()
        >= 3
}

fn append_depadded_status(output: &mut Vec<u8>, line: &[u8], status: &[u8]) {
    let mut end = line.len() - status.len();
    while end > 0 && line[end - 1] == b'.' {
        end -= 1;
    }
    output.extend_from_slice(line[..end].trim_ascii_end());
    output.push(b' ');
    append_line(output, status);
}
