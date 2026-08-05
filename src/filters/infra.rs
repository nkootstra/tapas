use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterDecision, StreamFilterOutput,
    append_line, command_basename, data::compact_json, find_subslice, normalize_log_line,
    strip_ansi_csi as strip_ansi, timestamp_end,
};

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    argv.first()
        .copied()
        .map(command_basename)
        .is_some_and(|command| {
            matches!(
                command,
                b"curl" | b"docker" | b"docker-compose" | b"kubectl" | b"gh" | b"acli"
            )
        })
}

pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    dispatch_streams_decision(argv, stdout, stderr, exit_code, lossless)
        .map(|decision| decision.into_output(stdout, stderr))
}

pub(crate) fn dispatch_streams_decision(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterDecision, FilterError> {
    let Some(command) = argv.first().copied().map(command_basename) else {
        return Err(FilterError::InvalidInput);
    };
    if lossless || crate::invocation_policy::requests_passthrough(argv) {
        return Ok(StreamFilterDecision::Unchanged);
    }
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let arg2 = argv.get(2).copied().unwrap_or_default();
    let arg3 = argv.get(3).copied().unwrap_or_default();

    if exit_code != 0 && !stderr.is_empty() {
        return Ok(StreamFilterDecision::Unchanged);
    }

    if command == b"curl" && has_verbose_flag(argv) {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            stdout.to_vec(),
            if stderr.is_empty() {
                Vec::new()
            } else {
                compact_curl(b"", stderr)
            },
            EvidenceClass::FactComplete,
        )));
    }
    if is_logs_invocation(command, argv) {
        let compose = command == b"docker-compose" || command == b"docker" && arg1 == b"compose";
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            EvidenceClass::FactComplete,
            |stdout, stderr| compact_logs(stdout, stderr, compose),
        ));
    }

    let output = if command == b"acli"
        && argv[1..]
            .iter()
            .take_while(|argument| **argument != b"--")
            .any(|argument| *argument == b"--json" || argument.starts_with(b"--json="))
    {
        compact_json(stdout)
    } else if is_docker_ps(command, argv) && matches_docker_ps(stdout) {
        Some(compact_docker_ps(stdout))
    } else if is_docker_images(command, argv) && matches_docker_images(stdout) {
        Some(compact_docker_images(stdout))
    } else if command == b"kubectl" && matches_kubectl(stdout) {
        Some(compact_kubectl(stdout))
    } else if command == b"gh" {
        Some(compact_gh(argv, stdout))
    } else if command == b"acli" {
        compact_acli(arg1, arg2, arg3, stdout)
    } else {
        None
    };

    Ok(output.map_or_else(
        || StreamFilterDecision::Unchanged,
        |stdout| {
            StreamFilterDecision::Applied(StreamFilterOutput::new(
                stdout,
                stderr.to_vec(),
                EvidenceClass::FactComplete,
            ))
        },
    ))
}

pub fn matches_container_pipe(input: &[u8]) -> bool {
    matches_kubectl(input) || matches_docker_ps(input)
}

pub fn apply_container_pipe(input: &[u8]) -> Result<FilterOutput, FilterError> {
    let bytes = if matches_kubectl(input) {
        compact_kubectl(input)
    } else if matches_docker_ps(input) {
        compact_docker_ps(input)
    } else {
        return Err(FilterError::InvalidInput);
    };
    Ok(FilterOutput::new(bytes, EvidenceClass::FactComplete))
}

pub fn matches_curl_pipe(input: &[u8]) -> bool {
    input
        .split(|byte| *byte == b'\n')
        .any(|line| matches!(line.first(), Some(b'*' | b'>' | b'<')))
}

pub fn apply_curl_pipe(input: &[u8]) -> Result<FilterOutput, FilterError> {
    if !matches_curl_pipe(input) {
        return Err(FilterError::InvalidInput);
    }
    Ok(FilterOutput::new(
        compact_curl_trace(input),
        EvidenceClass::FactComplete,
    ))
}

mod atlassian;
mod containers;
mod curl;
mod github;
mod logs;
mod table;

use atlassian::compact_acli;
use containers::{
    compact_docker_images, compact_docker_ps, compact_kubectl, is_docker_images,
    matches_docker_images, matches_kubectl,
};
use curl::{compact_curl, compact_curl_trace, has_verbose_flag};
use github::compact_gh;
use logs::{compact_logs, is_docker_ps, is_logs_invocation, matches_docker_ps};
