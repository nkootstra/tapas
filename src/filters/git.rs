use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterDecision, StreamFilterInput,
    StreamFilterOutput, find_subslice, rfind_subslice, strip_ansi,
};

pub fn matches(input: &[u8]) -> bool {
    matches_status(input)
        || matches_branch(input)
        || matches_reflog(input)
        || matches_show(input)
        || matches_diff(input)
        || matches_log(input)
        || matches_commit(input)
        || matches_merge(input)
        || matches_blame(input)
}

pub fn apply_matched(input: &[u8]) -> Result<FilterOutput, FilterError> {
    try_apply_matched(input)?.ok_or(FilterError::InvalidInput)
}

pub(crate) fn try_apply_matched(input: &[u8]) -> Result<Option<FilterOutput>, FilterError> {
    if matches_status(input) {
        return Ok(Some(FilterOutput::new(
            apply_status(input),
            EvidenceClass::FactComplete,
        )));
    }
    if matches_branch(input) {
        return Ok(Some(FilterOutput::new(
            apply_branch(input),
            EvidenceClass::FactComplete,
        )));
    }
    if matches_reflog(input) {
        return Ok(Some(FilterOutput::new(
            apply_reflog(input),
            EvidenceClass::FactComplete,
        )));
    }
    if matches_show(input) {
        return Ok(Some(FilterOutput::new(
            apply_show(input),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if matches_diff(input) {
        return Ok(Some(FilterOutput::new(
            apply_diff(input),
            EvidenceClass::FactComplete,
        )));
    }
    if matches_log(input) {
        return Ok(Some(FilterOutput::new(
            apply_log_compact(input),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if matches_commit(input) {
        return Ok(Some(FilterOutput::new(
            apply_commit(input),
            EvidenceClass::FactComplete,
        )));
    }
    if matches_merge(input) {
        return Ok(Some(FilterOutput::new(
            apply_merge(input, b""),
            EvidenceClass::FactComplete,
        )));
    }
    if matches_blame(input) {
        return Ok(Some(FilterOutput::new(
            apply_blame(input),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    Ok(None)
}

pub fn dispatch_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    _stderr: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<FilterOutput, FilterError> {
    try_dispatch_argv(argv, stdout, exit_code, lossless)
        .map(|output| output.unwrap_or_else(|| passthrough(stdout)))
}

fn try_dispatch_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<Option<FilterOutput>, FilterError> {
    if argv.len() < 2 {
        return Err(FilterError::InvalidInput);
    }
    if lossless || exit_code != 0 {
        return Ok(None);
    }

    match argv[1] {
        b"status" => {
            let args = &argv[1..];
            if has_arg(args, b"--porcelain") || has_arg(args, b"-z") {
                Ok(None)
            } else if has_arg(args, b"--short") || has_arg(args, b"-s") {
                Ok(Some(FilterOutput::new(
                    apply_status_short(stdout),
                    EvidenceClass::FactComplete,
                )))
            } else {
                Ok(Some(FilterOutput::new(
                    apply_status(stdout),
                    EvidenceClass::FactComplete,
                )))
            }
        }
        b"diff" => {
            let args = &argv[1..];
            if [
                b"--stat".as_slice(),
                b"--shortstat",
                b"--name-only",
                b"--name-status",
                b"--compact-summary",
                b"--summary",
                b"--patch-with-stat",
            ]
            .iter()
            .any(|argument| has_arg(args, argument))
            {
                Ok(None)
            } else {
                Ok(Some(FilterOutput::new(
                    apply_diff(stdout),
                    EvidenceClass::FactComplete,
                )))
            }
        }
        b"log" => {
            let args = &argv[1..];
            let custom = [
                b"--oneline".as_slice(),
                b"--name-only",
                b"--name-status",
                b"--compact-summary",
                b"--no-walk",
                b"--abbrev-commit",
                b"--graph",
                b"-p",
                b"--patch",
                b"-u",
            ]
            .iter()
            .any(|argument| has_arg(args, argument))
                || has_format_or_pretty_arg(args);
            if custom {
                Ok(None)
            } else if has_arg(args, b"--stat") || has_arg(args, b"--shortstat") {
                Ok(Some(FilterOutput::new(
                    apply_log_stat_compact(stdout),
                    EvidenceClass::PotentiallyLossy,
                )))
            } else {
                Ok(Some(FilterOutput::new(
                    apply_log_compact(stdout),
                    EvidenceClass::PotentiallyLossy,
                )))
            }
        }
        b"show" => {
            let args = &argv[1..];
            let summary = [
                b"--name-only".as_slice(),
                b"--name-status",
                b"--compact-summary",
                b"--no-patch",
                b"--raw",
                b"-s",
            ]
            .iter()
            .any(|argument| has_arg(args, argument));
            let blob = argv[2..]
                .iter()
                .any(|argument| !argument.starts_with(b"-") && argument.contains(&b':'));
            if summary || has_format_or_pretty_arg(args) || blob {
                Ok(None)
            } else if has_arg(args, b"--stat") || has_arg(args, b"--shortstat") {
                Ok(Some(FilterOutput::new(
                    apply_log_stat_compact(stdout),
                    EvidenceClass::PotentiallyLossy,
                )))
            } else {
                Ok(Some(FilterOutput::new(
                    apply_show(stdout),
                    EvidenceClass::PotentiallyLossy,
                )))
            }
        }
        b"branch" => Ok(Some(FilterOutput::new(
            apply_branch(stdout),
            EvidenceClass::FactComplete,
        ))),
        b"reflog" => {
            if has_format_or_pretty_arg(&argv[1..]) || !matches_reflog(stdout) {
                Ok(None)
            } else {
                Ok(Some(FilterOutput::new(
                    apply_reflog(stdout),
                    EvidenceClass::FactComplete,
                )))
            }
        }
        b"commit" => Ok(Some(FilterOutput::new(
            apply_commit(stdout),
            EvidenceClass::FactComplete,
        ))),
        b"merge" => Ok(Some(FilterOutput::new(
            apply_merge(stdout, b""),
            EvidenceClass::FactComplete,
        ))),
        b"blame" => {
            let args = &argv[1..];
            let alternate = [
                b"-s".as_slice(),
                b"--porcelain",
                b"-p",
                b"--line-porcelain",
                b"--incremental",
                b"-e",
                b"--show-email",
            ]
            .iter()
            .any(|argument| has_arg(args, argument));
            if alternate {
                Ok(None)
            } else {
                Ok(Some(FilterOutput::new(
                    apply_blame(stdout),
                    EvidenceClass::PotentiallyLossy,
                )))
            }
        }
        b"add" => Ok(Some(FilterOutput::new(
            apply_add(stdout, b""),
            EvidenceClass::FactComplete,
        ))),
        b"checkout" | b"switch" => Ok(Some(FilterOutput::new(
            apply_checkout(stdout, b""),
            EvidenceClass::FactComplete,
        ))),
        b"fetch" => Ok(Some(FilterOutput::new(
            apply_fetch(b""),
            EvidenceClass::FactComplete,
        ))),
        b"pull" => Ok(Some(FilterOutput::new(
            apply_pull(stdout, b""),
            EvidenceClass::FactComplete,
        ))),
        b"push" => Ok(Some(FilterOutput::new(
            apply_push(b""),
            EvidenceClass::FactComplete,
        ))),
        b"rebase" => Ok(Some(FilterOutput::new(
            apply_rebase(stdout, b""),
            EvidenceClass::FactComplete,
        ))),
        b"stash" => Ok(Some(FilterOutput::new(
            apply_stash(stdout, b""),
            EvidenceClass::FactComplete,
        ))),
        b"config" => {
            let args = &argv[1..];
            if (!has_arg(args, b"--list") && !has_arg(args, b"-l"))
                || has_arg(args, b"--null")
                || has_arg(args, b"-z")
            {
                Ok(None)
            } else {
                Ok(Some(FilterOutput::new(
                    compact_trimmed_lines(stdout),
                    EvidenceClass::FactComplete,
                )))
            }
        }
        // `git grep` emits one match per line; the output is already compact
        // and actionable, so it is intentionally left byte-exact.
        b"grep" => Ok(None),
        b"remote" => match compact_remote(&argv[1..], stdout) {
            Some(output) => Ok(Some(FilterOutput::new(output, EvidenceClass::FactComplete))),
            None => Ok(None),
        },
        b"shortlog" => Ok(Some(FilterOutput::new(
            compact_shortlog(stdout),
            EvidenceClass::FactComplete,
        ))),
        b"tag" => {
            if has_format_or_pretty_arg(&argv[1..]) || has_arg(&argv[1..], b"--column") {
                Ok(None)
            } else {
                Ok(Some(FilterOutput::new(
                    compact_trimmed_lines(stdout),
                    EvidenceClass::FactComplete,
                )))
            }
        }
        b"worktree" => {
            if has_arg(&argv[1..], b"--porcelain") || has_arg(&argv[1..], b"-z") {
                Ok(None)
            } else {
                Ok(Some(FilterOutput::new(
                    compact_worktree(stdout),
                    EvidenceClass::FactComplete,
                )))
            }
        }
        _ => Ok(None),
    }
}

/// Apply Git wrapper semantics while retaining each fact on its source stream.
/// Compactors that need both streams fail open when both contain output.
pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    argv.len() > 1
        && crate::catalog::filter_family_handles(argv, crate::catalog::GIT_FILTER_COMMANDS)
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
    if argv.len() < 2 {
        return Err(FilterError::InvalidInput);
    }
    if lossless || exit_code != 0 {
        return Ok(StreamFilterDecision::Unchanged);
    }
    if argv[1] == b"fetch" && !stdout.is_empty() {
        return Ok(StreamFilterDecision::Unchanged);
    }

    if argv[1] == b"pull" {
        let compact_stdout = compact_pull_stdout(stdout);
        let compact_stderr = compact_pull_stderr(stderr);
        if compact_stdout
            .as_deref()
            .is_none_or(|bytes| bytes == stdout)
            && compact_stderr
                .as_deref()
                .is_none_or(|bytes| bytes == stderr)
        {
            return Ok(StreamFilterDecision::Unchanged);
        }
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact_stdout.unwrap_or_else(|| stdout.to_vec()),
            compact_stderr.unwrap_or_else(|| stderr.to_vec()),
            EvidenceClass::FactComplete,
        )));
    }
    if argv[1] == b"push" {
        let compact_stdout = compact_push_stdout(stdout);
        let compact_stderr = compact_push_stderr(stderr);
        if compact_stdout
            .as_deref()
            .is_none_or(|bytes| bytes == stdout)
            && compact_stderr
                .as_deref()
                .is_none_or(|bytes| bytes == stderr)
        {
            return Ok(StreamFilterDecision::Unchanged);
        }
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact_stdout.unwrap_or_else(|| stdout.to_vec()),
            compact_stderr.unwrap_or_else(|| stderr.to_vec()),
            EvidenceClass::FactComplete,
        )));
    }

    type Compact = fn(&[u8], &[u8]) -> Vec<u8>;
    let compact: Option<Compact> = match argv[1] {
        b"add" => Some(apply_add),
        b"checkout" | b"switch" => Some(apply_checkout),
        b"fetch" => Some(|_, stderr| apply_fetch(stderr)),
        b"merge" => Some(apply_merge),
        b"rebase" => Some(apply_rebase),
        b"stash" => Some(apply_stash),
        _ => None,
    };
    if let Some(compact) = compact {
        if !stdout.is_empty() && !stderr.is_empty() {
            return Ok(StreamFilterDecision::Unchanged);
        }
        let output = StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            EvidenceClass::FactComplete,
            compact,
        );
        return Ok(applied_or_unchanged(output, stdout, stderr));
    }

    let Some(filtered) = try_dispatch_argv(argv, stdout, exit_code, lossless)? else {
        return Ok(StreamFilterDecision::Unchanged);
    };
    if filtered.bytes == stdout {
        return Ok(StreamFilterDecision::Unchanged);
    }
    Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
        filtered.bytes,
        stderr.to_vec(),
        filtered.evidence,
    )))
}

fn applied_or_unchanged(
    output: StreamFilterOutput,
    stdout: &[u8],
    stderr: &[u8],
) -> StreamFilterDecision {
    if output.stdout == stdout && output.stderr == stderr {
        StreamFilterDecision::Unchanged
    } else {
        StreamFilterDecision::Applied(output)
    }
}

mod blame;
mod commit;
mod diff;
mod log;
mod merge;
mod refs;
mod status;
mod transport;
mod wrapper;

use blame::{apply_blame, matches_blame};
use commit::{apply_commit, matches_commit};
use diff::apply_diff;
use log::{apply_log_compact, apply_log_stat_compact, apply_show, matches_log, matches_show};
use merge::{apply_merge, matches_merge};
use refs::{
    apply_branch, apply_reflog, compact_remote, compact_shortlog, compact_trimmed_lines,
    compact_worktree, has_arg, has_format_or_pretty_arg, matches_branch, matches_diff,
    matches_reflog, passthrough,
};
use status::{apply_status, matches_status};
use transport::{
    apply_fetch, apply_pull, apply_push, compact_pull_stderr, compact_pull_stdout,
    compact_push_stderr, compact_push_stdout,
};
use wrapper::{apply_add, apply_checkout, apply_rebase, apply_stash, apply_status_short};
