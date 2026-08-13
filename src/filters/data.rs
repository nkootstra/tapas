use super::{
    EvidenceClass, FilterError, StreamFilterDecision, StreamFilterInput, StreamFilterOutput,
    command_basename, find_subslice, strip_ansi,
};

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    crate::catalog::filter_family_handles(argv, crate::catalog::DATA_FILTER_COMMANDS)
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
    if argv.is_empty() {
        return Err(FilterError::InvalidInput);
    }
    let command = command_basename(argv[0]);
    let aws_table_route = command == b"aws" && table::aws_requests_table(argv);
    if lossless || crate::invocation_policy::requests_passthrough(argv) && !aws_table_route {
        return Ok(StreamFilterDecision::Unchanged);
    }

    if command == b"aws" && exit_code == 0 && aws_table_route && table::matches_aws_table(stdout) {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            table::compact_aws_table(stdout),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if command == b"psql"
        && exit_code == 0
        && table::is_psql_table_route(argv)
        && table::matches_psql_table(stdout)
    {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            table::compact_psql_table(stdout),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }

    if matches!(command, b"pup" | b"acli") && exit_code != 0 {
        return Ok(StreamFilterDecision::Unchanged);
    }
    let wants_json = matches!(command, b"jq" | b"pup" | b"acli")
        || command == b"gh" && gh_wants_data_output(argv);
    if wants_json
        && stderr.is_empty()
        && let Some(compact) = compact_json(stdout)
    {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact,
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        )));
    }

    if command == b"pup" && matches_pup_table(stdout) {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact_pup_table(stdout),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }

    if command == b"cat"
        && stdout.len() > 512
        && let Some(compact) = compact_cat(stdout, argv)
    {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact,
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }

    if matches!(command, b"bat" | b"batcat")
        && exit_code == 0
        && stderr.is_empty()
        && stdout.len() > 512
        && std::str::from_utf8(stdout).is_ok()
        && bat_is_plain(argv)
        && let Some(compact) = compact_cat(stdout, argv)
    {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact,
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        )));
    }

    if command == b"sqlite3" && matches_sqlite_table(stdout) {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact_sqlite_table(stdout),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }

    if is_columnar_command(command) && matches_columnar(stdout) {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact_columnar(stdout),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }

    Ok(StreamFilterDecision::Unchanged)
}

fn bat_is_plain(argv: &[&[u8]]) -> bool {
    argv.len() <= 2
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Language {
    Rust,
    Zig,
    Go,
    Python,
    TypeScript,
    JavaScript,
    Java,
    Cpp,
    Ruby,
    Data,
    Unknown,
}

mod cat;
mod exact;
mod json;
mod table;

use cat::compact_cat;
use exact::gh_wants_data_output;
pub use exact::{sigil_rle, ws_rle};
pub(crate) use json::compact_json;
use table::{
    compact_columnar, compact_pup_table, compact_sqlite_table, is_columnar_command,
    matches_columnar, matches_pup_table, matches_sqlite_table,
};
