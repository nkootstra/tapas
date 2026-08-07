use super::{
    EvidenceClass, FilterError, StreamFilterDecision, StreamFilterInput, StreamFilterOutput,
    command_basename,
};

mod lint;
mod mypy;
mod plan;
mod precommit;
mod prettier;
mod ruff;

use lint::{compact_lint, matches_lint};
use mypy::compact_mypy;
use plan::{compact_plan, matches_plan};
use precommit::compact_precommit;
use prettier::compact_prettier;
use ruff::compact_ruff;

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    crate::catalog::filter_family_handles(argv, crate::catalog::DIAGNOSTICS_FILTER_COMMANDS)
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
        exit_code: _,
        lossless,
    } = input;
    let Some(command) = argv.first().copied().map(command_basename) else {
        return Err(FilterError::InvalidInput);
    };
    if lossless || crate::invocation_policy::requests_passthrough(argv) {
        return Ok(StreamFilterDecision::Unchanged);
    }

    type Compact = fn(&[u8], &[u8]) -> Vec<u8>;
    let output: Option<(Compact, EvidenceClass)> = match command {
        b"mypy" => Some((compact_mypy, EvidenceClass::FactComplete)),
        b"ruff" => Some((compact_ruff, EvidenceClass::FactComplete)),
        b"eslint" | b"biome" if matches_lint(stdout) || matches_lint(stderr) => {
            Some((compact_lint, EvidenceClass::FactComplete))
        }
        b"pre-commit" => Some((compact_precommit, EvidenceClass::FactComplete)),
        b"prettier" => Some((compact_prettier, EvidenceClass::FactComplete)),
        b"terraform" | b"tofu"
            if argv.get(1).copied() == Some(b"plan")
                && (matches_plan(stdout) || matches_plan(stderr)) =>
        {
            Some((compact_plan, EvidenceClass::FactComplete))
        }
        _ => None,
    };

    Ok(output.map_or_else(
        || StreamFilterDecision::Unchanged,
        |(compact, evidence)| {
            StreamFilterDecision::compact_single_stream(stdout, stderr, evidence, compact)
        },
    ))
}
