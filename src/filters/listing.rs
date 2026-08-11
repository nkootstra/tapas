use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterDecision, StreamFilterInput,
    StreamFilterOutput, command_basename,
};
use crate::filters::generic;

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    crate::catalog::filter_family_handles(argv, crate::catalog::LISTING_FILTER_COMMANDS)
}

pub fn matches(input: &[u8]) -> bool {
    matches_tree(input) || matches_ls_long(input) || matches_find_ls(input) || matches_du(input)
}

pub fn apply_matched(input: &[u8]) -> Result<FilterOutput, FilterError> {
    try_apply_matched(input)?.ok_or(FilterError::InvalidInput)
}

pub(crate) fn try_apply_matched(input: &[u8]) -> Result<Option<FilterOutput>, FilterError> {
    if matches_tree(input) {
        return Ok(Some(FilterOutput::new(
            apply_tree_pipe(input),
            EvidenceClass::FactComplete,
        )));
    }
    if matches_ls_long(input) {
        let bytes = apply_ls_long(input).ok_or(FilterError::InvalidInput)?;
        return Ok(Some(FilterOutput::new(
            bytes,
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if matches_find_ls(input) {
        return Ok(Some(FilterOutput::new(
            apply_find_ls(input),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if matches_du(input) {
        return Ok(Some(FilterOutput::new(
            apply_du(input, true),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    Ok(None)
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
    if argv.is_empty() {
        return Err(FilterError::InvalidInput);
    }
    if lossless || crate::invocation_policy::requests_passthrough(argv) {
        return Ok(StreamFilterDecision::Unchanged);
    }
    let command = command_basename(argv[0]);
    if command == b"find" {
        if matches_find_plain(stdout) {
            return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
                apply_find_plain(stdout, find_has_type_file(argv)),
                stderr.to_vec(),
                EvidenceClass::PotentiallyLossy,
            )));
        }
        return Ok(StreamFilterDecision::Unchanged);
    }
    if command == b"tree" {
        if !matches_tree(stdout) {
            return Ok(StreamFilterDecision::Unchanged);
        }
        let Some(compact) = apply_tree_compact(stdout) else {
            return Ok(StreamFilterDecision::Unchanged);
        };
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact,
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if command == b"ls" {
        let compact = if matches_ls_long(stdout) {
            apply_ls_long(stdout)
        } else {
            apply_ls_plain(stdout, ls_wants_columns(argv))
        };
        let Some(compact) = compact else {
            return Ok(StreamFilterDecision::Unchanged);
        };
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact,
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if command == b"du" {
        if !matches_du(stdout) {
            return Ok(StreamFilterDecision::Unchanged);
        }
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            apply_du(stdout, du_has_summarize(argv)),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if command == b"wc" {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            apply_wc(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::FactComplete,
        )));
    }
    if command == b"env" && env_is_listing(argv) {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            apply_env(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
    }
    if command == b"rg" {
        if rg_is_file_mode(argv) && matches_rg_files(stdout) {
            return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
                apply_rg_files(stdout),
                stderr.to_vec(),
                EvidenceClass::FactComplete,
            )));
        }
        if matches_rg_pattern(stdout) {
            return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
                apply_rg_pattern(stdout),
                stderr.to_vec(),
                EvidenceClass::FactComplete,
            )));
        }
    }
    if is_text_filter_command(command)
        && (generic::matches(stdout) || generic::matches(stderr))
    {
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            EvidenceClass::PotentiallyLossy,
            compact_text_output,
        ));
    }
    Ok(StreamFilterDecision::Unchanged)
}

fn is_text_filter_command(command: &[u8]) -> bool {
    crate::catalog::TEXT_FILTER_COMMANDS.contains(&command)
}

fn compact_text_output(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let input = if !stdout.is_empty() { stdout } else { stderr };
    if !generic::matches(input) {
        return input.to_vec();
    }
    generic::apply_matched(input)
        .ok()
        .map(|output| output.bytes)
        .unwrap_or_else(|| input.to_vec())
}

mod du;
mod find;
mod ls;
mod pipe;
mod rg;
mod shell;
mod tree;
mod tree_pipe;

use du::{apply_du, matches_du};
use find::{apply_find_plain, find_has_type_file, matches_find_plain};
use ls::{apply_ls_plain, ls_wants_columns};
use pipe::{apply_find_ls, apply_ls_long, matches_find_ls, matches_ls_long, matches_tree};
use rg::{apply_rg_files, apply_rg_pattern, matches_rg_files, matches_rg_pattern, rg_is_file_mode};
use shell::{apply_env, apply_wc, du_has_summarize, env_is_listing};
use tree::apply_tree_compact;
use tree_pipe::apply_tree_pipe;
