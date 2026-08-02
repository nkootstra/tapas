use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterOutput, append_line, byte_after_lines,
    command_basename, find_subslice, strip_ansi, trim_ascii_end_space as trim_end,
};

const TREE_PREFIXES: &[&[u8]] = &[
    b"\xe2\x94\x9c\xe2\x94\x80\xe2\x94\x80 ",
    b"\xe2\x94\x94\xe2\x94\x80\xe2\x94\x80 ",
    b"\xe2\x94\x9c\xe2\x94\x80\xe2\x94\xac ",
    b"\xe2\x94\x94\xe2\x94\x80\xe2\x94\xac ",
    b"\xe2\x94\x9c\xe2\x94\x80 ",
    b"\xe2\x94\x94\xe2\x94\x80 ",
];

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    argv.first()
        .copied()
        .map(command_basename)
        .is_some_and(|command| {
            matches!(
                command,
                b"npm" | b"pnpm" | b"yarn" | b"bun" | b"composer" | b"pip" | b"pip3"
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
    if argv.is_empty() {
        return Err(FilterError::InvalidInput);
    }
    if lossless || requests_exact_output(argv) {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }

    let command = command_basename(argv[0]);
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let arg2 = argv.get(2).copied().unwrap_or_default();
    let recognized_error = has_package_error_marker(stdout) || has_package_error_marker(stderr);
    if exit_code != 0 && !stderr.is_empty() && !recognized_error {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }

    let package_tree_route = command == b"bun" && arg1 == b"pm" && arg2 == b"ls"
        || matches!(command, b"npm" | b"pnpm") && matches!(arg1, b"ls" | b"list")
        || command == b"yarn" && arg1 == b"list";
    if package_tree_route && matches_package_tree(stdout) {
        return Ok(StreamFilterOutput::new(
            compact_package_tree(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }

    let js_install_route = matches!(command, b"npm" | b"pnpm" | b"yarn" | b"bun")
        && matches!(arg1, b"install" | b"i" | b"ci" | b"add" | b"remove" | b"rm");
    let composer_route = command == b"composer"
        && matches!(
            arg1,
            b"install" | b"require" | b"update" | b"upgrade" | b"remove" | b"create-project"
        );
    if (js_install_route || composer_route)
        && (matches_npm_install(stdout) || matches_npm_install(stderr) || recognized_error)
    {
        let evidence = if exit_code == 0 {
            EvidenceClass::PotentiallyLossy
        } else {
            EvidenceClass::FactComplete
        };
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            evidence,
            compact_npm_install,
        ));
    }

    let pip_command = matches!(command, b"pip" | b"pip3");
    let pip_table_route = pip_command && matches!(arg1, b"list" | b"outdated");
    if pip_table_route {
        return Ok(StreamFilterOutput::new(
            compact_pip_table(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::FactComplete,
        ));
    }
    let pip_install_route = pip_command && matches!(arg1, b"install" | b"download" | b"wheel");
    if pip_install_route
        && (looks_like_pip_install(stdout) || looks_like_pip_install(stderr) || recognized_error)
    {
        let evidence = if exit_code == 0 {
            EvidenceClass::PotentiallyLossy
        } else {
            EvidenceClass::FactComplete
        };
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            evidence,
            compact_pip,
        ));
    }

    Ok(StreamFilterOutput::passthrough(stdout, stderr))
}

pub fn matches_pipe(input: &[u8]) -> bool {
    matches_npm_install(input)
}

pub fn apply_pipe(input: &[u8]) -> Result<FilterOutput, FilterError> {
    if !matches_pipe(input) {
        return Err(FilterError::InvalidInput);
    }
    Ok(FilterOutput::new(
        compact_npm_install(input, b""),
        EvidenceClass::PotentiallyLossy,
    ))
}

mod bun_yarn;
mod exact;
mod npm;
mod npm_install;
mod pip;
mod pnpm;
mod tree;

use bun_yarn::*;
use exact::*;
use npm::*;
use npm_install::*;
use pip::*;
use pnpm::*;
use tree::*;
