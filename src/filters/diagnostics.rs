use super::{
    EvidenceClass, FilterError, StreamFilterDecision, StreamFilterInput, StreamFilterOutput,
    command_basename,
};

mod golangci_lint;
mod lint;
mod mypy;
mod plan;
mod precommit;
mod prettier;
mod rubocop;
mod ruff;

use golangci_lint::classify_golangci_lint;
use lint::{compact_lint, matches_lint};
use mypy::classify_mypy;
use plan::{compact_plan, matches_plan};
use precommit::compact_precommit;
use prettier::compact_prettier;
use rubocop::classify_rubocop;
use ruff::classify_ruff;

pub(super) enum RecognizedStream {
    Diagnostics(Vec<u8>),
    Clean(Vec<u8>),
}

pub(super) fn split_location(line: &[u8], require_column: bool) -> Option<(&[u8], &[u8], &[u8])> {
    for separator in line
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b':' && index > 0).then_some(index))
    {
        let line_number = &line[separator + 1..];
        let line_digits = line_number
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if line_digits == 0 || line_number.get(line_digits) != Some(&b':') {
            continue;
        }

        let after_line = &line_number[line_digits + 1..];
        let column_digits = after_line
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if column_digits > 0 && after_line.get(column_digits) == Some(&b':') {
            let location_end = separator + 1 + line_digits + 1 + column_digits;
            return Some((
                &line[..separator],
                &line[separator + 1..location_end],
                &line[location_end + 1..],
            ));
        }
        if !require_column {
            let location_end = separator + 1 + line_digits;
            return Some((
                &line[..separator],
                &line[separator + 1..location_end],
                after_line,
            ));
        }
    }
    None
}

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
        exit_code,
        lossless,
    } = input;
    let Some(command) = argv.first().copied().map(command_basename) else {
        return Err(FilterError::InvalidInput);
    };
    if lossless || crate::invocation_policy::requests_passthrough(argv) {
        return Ok(StreamFilterDecision::Passthrough);
    }

    match command {
        b"mypy" => {
            return Ok(strict_route(
                stdout,
                stderr,
                exit_code,
                false,
                classify_mypy,
            ));
        }
        b"ruff" => {
            return Ok(strict_route(
                stdout,
                stderr,
                exit_code,
                false,
                classify_ruff,
            ));
        }
        b"golangci-lint" if argv.get(1).copied() == Some(b"run") => {
            return Ok(strict_route(
                stdout,
                stderr,
                exit_code,
                true,
                classify_golangci_lint,
            ));
        }
        b"golangci-lint" => return Ok(StreamFilterDecision::Passthrough),
        b"rubocop" => {
            return Ok(strict_route(
                stdout,
                stderr,
                exit_code,
                false,
                classify_rubocop,
            ));
        }
        _ => {}
    }

    type Compact = fn(&[u8], &[u8]) -> Vec<u8>;
    let output: Option<(Compact, EvidenceClass)> = match command {
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

fn strict_route(
    stdout: &[u8],
    stderr: &[u8],
    exit_code: i32,
    empty_success_is_clean: bool,
    classify: fn(&[u8]) -> Option<RecognizedStream>,
) -> StreamFilterDecision {
    if std::str::from_utf8(stdout).is_err()
        || std::str::from_utf8(stderr).is_err()
        || looks_structured(stdout)
        || looks_structured(stderr)
    {
        return StreamFilterDecision::Passthrough;
    }

    let stdout_recognized = classify(stdout);
    let stderr_recognized = classify(stderr);
    let has_diagnostics = matches!(stdout_recognized, Some(RecognizedStream::Diagnostics(_)))
        || matches!(stderr_recognized, Some(RecognizedStream::Diagnostics(_)));
    let has_recognized = stdout_recognized.is_some() || stderr_recognized.is_some();

    if exit_code != 0 && !has_diagnostics {
        return StreamFilterDecision::Passthrough;
    }
    if !has_recognized {
        if empty_success_is_clean && exit_code == 0 && stdout.is_empty() && stderr.is_empty() {
            return StreamFilterDecision::Applied(StreamFilterOutput::new(
                Vec::new(),
                Vec::new(),
                EvidenceClass::FactComplete,
            ));
        }
        return StreamFilterDecision::Passthrough;
    }

    StreamFilterDecision::Applied(StreamFilterOutput::new(
        recognized_or_original(stdout_recognized, stdout),
        recognized_or_original(stderr_recognized, stderr),
        EvidenceClass::FactComplete,
    ))
}

fn recognized_or_original(recognized: Option<RecognizedStream>, original: &[u8]) -> Vec<u8> {
    match recognized {
        Some(RecognizedStream::Diagnostics(output) | RecognizedStream::Clean(output)) => output,
        None => original.to_vec(),
    }
}

fn looks_structured(input: &[u8]) -> bool {
    let input = input.trim_ascii_start();
    input.starts_with(b"{")
        || input.starts_with(b"[")
        || input.starts_with(b"<")
        || input.starts_with(b"---")
        || input.starts_with(b"%YAML")
        || input.starts_with(b"::error")
        || input.starts_with(b"::warning")
        || input.starts_with(b"::notice")
        || input.starts_with(b"##vso[task.logissue")
}
