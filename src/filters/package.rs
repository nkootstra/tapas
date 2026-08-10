use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterDecision, StreamFilterInput,
    StreamFilterOutput, append_line, byte_after_lines, command_basename, find_subslice, strip_ansi,
    trim_ascii_end_space as trim_end,
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
    crate::catalog::filter_family_handles(argv, crate::catalog::PACKAGE_FILTER_COMMANDS)
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
    if lossless
        || std::str::from_utf8(stdout).is_err()
        || std::str::from_utf8(stderr).is_err()
        || crate::invocation_policy::requests_passthrough(argv)
    {
        return Ok(StreamFilterDecision::Unchanged);
    }

    let command = command_basename(argv[0]);
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let arg2 = argv.get(2).copied().unwrap_or_default();
    let recognized_error = has_package_error_marker(stdout) || has_package_error_marker(stderr);
    if exit_code != 0 && !stderr.is_empty() && !recognized_error {
        return Ok(StreamFilterDecision::Unchanged);
    }

    let package_tree_route = command == b"bun" && arg1 == b"pm" && arg2 == b"ls"
        || matches!(command, b"npm" | b"pnpm") && matches!(arg1, b"ls" | b"list")
        || command == b"yarn" && arg1 == b"list";
    if package_tree_route && matches_package_tree(stdout) {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact_package_tree(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        )));
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
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            evidence,
            compact_npm_install,
        ));
    }

    let pip_command = matches!(command, b"pip" | b"pip3");
    if pip_command && exit_code != 0 {
        return Ok(StreamFilterDecision::Unchanged);
    }
    let pip_table_route = pip_command && matches!(arg1, b"list" | b"outdated");
    if pip_table_route {
        return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
            compact_pip_table(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::FactComplete,
        )));
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
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            evidence,
            compact_pip,
        ));
    }

    let uv_route = command == b"uv"
        && (matches!(arg1, b"sync" | b"add" | b"remove" | b"lock" | b"tree")
            || arg1 == b"pip" && matches!(arg2, b"install" | b"sync" | b"compile"));
    if exit_code == 0 && uv_route && matches_uv_output(stdout, stderr) {
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            EvidenceClass::PotentiallyLossy,
            compact_uv,
        ));
    }

    Ok(StreamFilterDecision::Unchanged)
}

fn matches_uv_output(stdout: &[u8], stderr: &[u8]) -> bool {
    [stdout, stderr].into_iter().any(|input| {
        input.split(|byte| *byte == b'\n').any(|line| {
            let line = trim_end(line);
            [
                b"Resolved ".as_slice(),
                b"Prepared ",
                b"Installed ",
                b"Uninstalled ",
                b"Audited ",
                b"error:",
                b"warning:",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
                || matches!(line.first(), Some(b'+' | b'-')) && find_subslice(line, b"==").is_some()
                || TREE_PREFIXES.iter().any(|prefix| line.starts_with(prefix))
        })
    })
}

fn compact_uv(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            let line = trim_end(raw);
            if line.is_empty() {
                continue;
            }
            if [
                b"Resolved ".as_slice(),
                b"Prepared ",
                b"Installed ",
                b"Uninstalled ",
                b"Audited ",
                b"error:",
                b"warning:",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
                || matches!(line.first(), Some(b'+' | b'-'))
                || TREE_PREFIXES.iter().any(|prefix| line.starts_with(prefix))
            {
                append_line(&mut output, line);
            }
        }
    }
    output
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

use npm::{has_package_error_marker, matches_package_tree};
use npm_install::compact_npm_install;
use pip::{compact_pip, compact_pip_table, looks_like_pip_install, matches_npm_install};
use tree::compact_package_tree;
