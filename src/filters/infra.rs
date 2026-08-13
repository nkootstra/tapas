use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterDecision, StreamFilterInput,
    StreamFilterOutput, append_line, command_basename, data::compact_json, find_subslice,
    normalize_log_line, strip_ansi_csi as strip_ansi, timestamp_end,
};

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    crate::catalog::filter_family_handles(argv, crate::catalog::INFRA_FILTER_COMMANDS)
}

pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    dispatch_streams_decision(StreamFilterInput::new(
        argv, stdout, stderr, exit_code, lossless,
    ))
    .map(|decision| decision.into_output(stdout, stderr))
}

pub(crate) fn dispatch_streams_decision(
    input: StreamFilterInput<'_>,
) -> Result<StreamFilterDecision, FilterError> {
    let StreamFilterInput {
        argv,
        stdout,
        stderr,
        exit_code,
        lossless,
    } = input;
    let Some(command) = argv.first().copied().map(command_basename) else {
        return Err(FilterError::InvalidInput);
    };
    if lossless || crate::invocation_policy::requests_passthrough(argv) {
        return Ok(StreamFilterDecision::Unchanged);
    }
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let arg2 = argv.get(2).copied().unwrap_or_default();
    let arg3 = argv.get(3).copied().unwrap_or_default();

    if command == b"helm"
        && exit_code == 0
        && stderr.is_empty()
        && std::str::from_utf8(stdout).is_ok()
        && let Some(stdout) = helm::compact(argv, stdout)
    {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            stdout,
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if matches!(command, b"docker" | b"docker-compose")
        && exit_code == 0
        && stderr.is_empty()
        && std::str::from_utf8(stdout).is_ok()
        && stats::route(command, argv)
        && let Some(stdout) = stats::compact(stdout)
    {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            stdout,
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        )));
    }

    let gh_pending_checks = command == b"gh"
        && arg1 == b"pr"
        && arg2 == b"checks"
        && exit_code == 8
        && stderr.is_empty();
    let gh_owned_failure = command == b"gh"
        && (arg1 == b"pr" && matches!(arg2, b"view" | b"checks")
            || arg1 == b"issue" && arg2 == b"view"
            || arg1 == b"run" && arg2 == b"list");
    if command == b"curl"
        && is_single_verbose_invocation(argv)
        && matches_classic_verbose_trace(stderr)
    {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            stdout.to_vec(),
            compact_curl(b"", stderr),
            EvidenceClass::FactComplete,
        )));
    }
    if exit_code != 0 && (!stderr.is_empty() || gh_owned_failure) && !gh_pending_checks {
        return Ok(StreamFilterDecision::Unchanged);
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
    } else if command == b"gh" && std::str::from_utf8(stdout).is_ok() {
        compact_gh(argv, stdout)
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
mod helm;
mod logs;
mod stats;
mod table;

use atlassian::compact_acli;
use containers::{
    compact_docker_images, compact_docker_ps, compact_kubectl, is_docker_images,
    matches_docker_images, matches_kubectl,
};
use curl::{
    compact_curl, compact_curl_trace, is_single_verbose_invocation, matches_classic_verbose_trace,
};
use github::compact_gh;
use logs::{compact_logs, is_docker_ps, is_logs_invocation, matches_docker_ps};
