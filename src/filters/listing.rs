use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterOutput, command_basename, find_subslice,
};

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    argv.first()
        .copied()
        .map(command_basename)
        .is_some_and(|command| {
            matches!(
                command,
                b"find" | b"tree" | b"ls" | b"du" | b"wc" | b"env" | b"rg"
            )
        })
}

pub fn matches(input: &[u8]) -> bool {
    matches_tree(input) || matches_ls_long(input) || matches_find_ls(input) || matches_du(input)
}

pub fn apply_matched(input: &[u8]) -> Result<FilterOutput, FilterError> {
    if matches_tree(input) {
        return Ok(FilterOutput::new(
            apply_tree_pipe(input),
            EvidenceClass::FactComplete,
        ));
    }
    if matches_ls_long(input) {
        let bytes = apply_ls_long(input).ok_or(FilterError::InvalidInput)?;
        return Ok(FilterOutput::new(bytes, EvidenceClass::PotentiallyLossy));
    }
    if matches_find_ls(input) {
        return Ok(FilterOutput::new(
            apply_find_ls(input),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if matches_du(input) {
        return Ok(FilterOutput::new(
            apply_du(input, true),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    Err(FilterError::InvalidInput)
}

pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    _exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    if argv.is_empty() {
        return Err(FilterError::InvalidInput);
    }
    if lossless {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }
    let command = command_basename(argv[0]);
    if requests_exact_query(argv) {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }
    if command == b"find" {
        if find_requests_exact(argv) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        if matches_find_plain(stdout) {
            return Ok(StreamFilterOutput::new(
                apply_find_plain(stdout, find_has_type_file(argv)),
                stderr.to_vec(),
                EvidenceClass::PotentiallyLossy,
            ));
        }
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }
    if command == b"tree" {
        if tree_requests_exact(argv) || !matches_tree(stdout) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        let Some(compact) = apply_tree_compact(stdout) else {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        };
        return Ok(StreamFilterOutput::new(
            compact,
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if command == b"ls" {
        if ls_requests_exact(argv) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        let compact = if matches_ls_long(stdout) {
            apply_ls_long(stdout)
        } else {
            apply_ls_plain(stdout, ls_wants_columns(argv))
        };
        let Some(compact) = compact else {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        };
        return Ok(StreamFilterOutput::new(
            compact,
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if command == b"du" {
        if !matches_du(stdout) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        return Ok(StreamFilterOutput::new(
            apply_du(stdout, du_has_summarize(argv)),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if command == b"wc" {
        return Ok(StreamFilterOutput::new(
            apply_wc(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::FactComplete,
        ));
    }
    if command == b"env" && env_is_listing(argv) {
        return Ok(StreamFilterOutput::new(
            apply_env(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if command == b"rg" {
        if rg_is_file_mode(argv) && matches_rg_files(stdout) {
            return Ok(StreamFilterOutput::new(
                apply_rg_files(stdout),
                stderr.to_vec(),
                EvidenceClass::FactComplete,
            ));
        }
        if rg_requests_exact(argv) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        if matches_rg_pattern(stdout) {
            return Ok(StreamFilterOutput::new(
                apply_rg_pattern(stdout),
                stderr.to_vec(),
                EvidenceClass::FactComplete,
            ));
        }
    }
    Ok(StreamFilterOutput::passthrough(stdout, stderr))
}

mod du;
mod find;
mod ls;
mod pipe;
mod rg;
mod shell;
mod tree;
mod tree_pipe;

use du::*;
use find::*;
use ls::*;
use pipe::*;
use rg::*;
use shell::*;
use tree::*;
use tree_pipe::*;
