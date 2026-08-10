use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterDecision, StreamFilterInput,
    StreamFilterOutput, append_line, command_basename, find_subslice, strip_ansi,
};

type Matcher = fn(&[u8]) -> bool;
type Apply = fn(&[u8], &[u8]) -> Vec<u8>;

const PIPE_FILTERS: &[(Matcher, Apply)] = &[
    (matches_cargo_test, apply_cargo_test),
    (matches_jest, apply_jest),
    (matches_js_test, apply_js_test),
    (matches_tsc, apply_tsc),
    (matches_go_test, apply_go_test),
    (matches_pytest, apply_pytest),
];

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    if !crate::catalog::filter_family_handles(argv, crate::catalog::TEST_TOOLS_FILTER_COMMANDS) {
        return false;
    }
    let Some(command) = argv.first().copied().map(command_basename) else {
        return false;
    };
    let arg1 = argv.get(1).copied().unwrap_or_default();
    matches!(
        command,
        b"pytest" | b"jest" | b"vitest" | b"mocha" | b"tsc" | b"ctest" | b"playwright"
    ) || command == b"cargo" && arg1 == b"test"
        || command == b"go" && arg1 == b"test"
        || command == b"node" && arg1 == b"--test"
        || matches!(command, b"npm" | b"pnpm" | b"yarn" | b"bun") && arg1 == b"test"
}

pub fn matches(input: &[u8]) -> bool {
    PIPE_FILTERS.iter().any(|(matches, _)| matches(input))
}

pub fn apply_matched(input: &[u8]) -> Result<FilterOutput, FilterError> {
    try_apply_matched(input)?.ok_or(FilterError::InvalidInput)
}

pub(crate) fn try_apply_matched(input: &[u8]) -> Result<Option<FilterOutput>, FilterError> {
    Ok(PIPE_FILTERS
        .iter()
        .find(|(matches, _)| matches(input))
        .map(|(_, apply)| FilterOutput::new(apply(input, b""), EvidenceClass::FactComplete)))
}

pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    _exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    dispatch_streams_decision(StreamFilterInput::new(
        argv, stdout, stderr, _exit_code, lossless,
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
    if argv.is_empty() {
        return Err(FilterError::InvalidInput);
    }
    if lossless
        || std::str::from_utf8(stdout).is_err()
        || std::str::from_utf8(stderr).is_err()
        || crate::invocation_policy::requests_passthrough(argv)
    {
        return Ok(StreamFilterDecision::Unchanged);
    }
    let command = command_basename(argv[0]);
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let script_test = arg1 == b"test" && matches!(command, b"npm" | b"pnpm" | b"yarn" | b"bun");

    if exit_code == 0 {
        if command == b"ctest"
            && catalog_routes::ctest_route(argv)
            && catalog_routes::matches_ctest(stdout, stderr)
        {
            return Ok(StreamFilterDecision::compact_single_stream(
                stdout,
                stderr,
                EvidenceClass::PotentiallyLossy,
                catalog_routes::compact_ctest,
            ));
        }
        if command == b"playwright"
            && catalog_routes::playwright_route(argv)
            && catalog_routes::matches_playwright(stdout, stderr)
        {
            return Ok(StreamFilterDecision::compact_single_stream(
                stdout,
                stderr,
                EvidenceClass::PotentiallyLossy,
                catalog_routes::compact_playwright,
            ));
        }
    }

    let compact: Option<Apply> = if command == b"pytest"
        && stream_matches(stdout, stderr, matches_pytest)
    {
        Some(apply_pytest)
    } else if command == b"cargo"
        && arg1 == b"test"
        && stream_matches(stdout, stderr, matches_cargo_test)
    {
        Some(apply_cargo_test)
    } else if (matches!(command, b"jest" | b"vitest") || script_test)
        && stream_matches(stdout, stderr, matches_jest)
    {
        Some(apply_jest)
    } else if (script_test || command == b"mocha" || (command == b"node" && arg1 == b"--test"))
        && stream_matches(stdout, stderr, matches_js_test)
    {
        Some(apply_js_test)
    } else if command == b"tsc" && stream_matches(stdout, stderr, matches_tsc) {
        Some(apply_tsc)
    } else if command == b"go" && arg1 == b"test" && stream_matches(stdout, stderr, matches_go_test)
    {
        Some(apply_go_test)
    } else {
        None
    };

    Ok(compact.map_or_else(
        || StreamFilterDecision::Unchanged,
        |compact| {
            StreamFilterDecision::compact_single_stream(
                stdout,
                stderr,
                EvidenceClass::FactComplete,
                compact,
            )
        },
    ))
}

fn stream_matches(stdout: &[u8], stderr: &[u8], matcher: fn(&[u8]) -> bool) -> bool {
    matcher(stdout) || matcher(stderr)
}

mod cargo;
mod catalog_routes;
mod go;
mod javascript;
mod jest;
mod pytest;
mod tsc;

use cargo::{apply_cargo_test, matches_cargo_test};
use go::{apply_go_test, matches_go_test};
use javascript::{apply_js_test, matches_js_test};
use jest::{apply_jest, matches_jest};
use pytest::{apply_pytest, matches_pytest};
use tsc::{apply_tsc, matches_tsc};
