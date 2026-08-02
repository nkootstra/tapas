pub(super) fn is_logs_invocation(command: &[u8], argv: &[&[u8]]) -> bool {
    command == b"kubectl" && argv.get(1).copied() == Some(b"logs")
        || command == b"docker" && argv.get(1).copied() == Some(b"logs")
        || command == b"docker-compose" && argv.get(1).copied() == Some(b"logs")
        || command == b"docker"
            && argv.get(1).copied() == Some(b"compose")
            && argv.get(2).copied() == Some(b"logs")
}

pub(super) fn compact_logs(stdout: &[u8], stderr: &[u8], compose: bool) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_logs(stdout, &mut output, compose);
    scan_logs(stderr, &mut output, compose);
    output
}

fn scan_logs(input: &[u8], output: &mut Vec<u8>, compose: bool) {
    let mut pending = Vec::new();
    let mut pending_fingerprint = Vec::new();
    let mut repeats = 0usize;
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if line.is_empty() {
            flush_log(output, &mut pending, &mut pending_fingerprint, &mut repeats);
            continue;
        }
        let normalized = normalize_log_line(line, compose);
        let fingerprint = if compose {
            normalized.clone()
        } else {
            normalized[timestamp_end(&normalized)..].to_vec()
        };
        if repeats > 0 && fingerprint == pending_fingerprint {
            repeats += 1;
            continue;
        }
        flush_log(output, &mut pending, &mut pending_fingerprint, &mut repeats);
        pending = normalized;
        pending_fingerprint = fingerprint;
        repeats = 1;
    }
    flush_log(output, &mut pending, &mut pending_fingerprint, &mut repeats);
}

fn flush_log(
    output: &mut Vec<u8>,
    pending: &mut Vec<u8>,
    fingerprint: &mut Vec<u8>,
    repeats: &mut usize,
) {
    if *repeats == 0 {
        return;
    }
    let start = timestamp_end(pending);
    output.extend_from_slice(if start > 0 {
        &pending[start..]
    } else {
        pending
    });
    if *repeats > 1 {
        output.extend_from_slice(" ×".as_bytes());
        output.extend_from_slice(repeats.to_string().as_bytes());
    }
    output.push(b'\n');
    pending.clear();
    fingerprint.clear();
    *repeats = 0;
}

pub(super) fn is_docker_ps(command: &[u8], argv: &[&[u8]]) -> bool {
    command == b"docker" && argv.get(1).copied() == Some(b"ps")
        || command == b"docker-compose" && argv.get(1).copied() == Some(b"ps")
        || command == b"docker"
            && argv.get(1).copied() == Some(b"compose")
            && argv.get(2).copied() == Some(b"ps")
}

pub(super) fn matches_docker_ps(input: &[u8]) -> bool {
    first_nonempty(input).is_some_and(|header| {
        header.starts_with(b"CONTAINER ID")
            || header.starts_with(b"NAME")
                && find_subslice(header, b"IMAGE").is_some()
                && find_subslice(header, b"SERVICE").is_some()
                && find_subslice(header, b"STATUS").is_some()
    })
}
use super::table::first_nonempty;
use super::{find_subslice, normalize_log_line, strip_ansi, timestamp_end};
