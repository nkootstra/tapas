use super::{
    EvidenceClass, FilterError, StreamFilterDecision, StreamFilterInput, StreamFilterOutput,
    append_line, byte_after_lines, command_basename, contains_ignore_ascii_case, find_subslice,
    trim_ascii_end_space as trim_ascii_end,
};

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    crate::catalog::filter_family_handles(argv, crate::catalog::BUILD_FILTER_COMMANDS)
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
    let runner_package_prelude = matches!(command, b"poetry" | b"pnpm" | b"npx" | b"bunx")
        && (has_package_prelude(stdout) || has_package_prelude(stderr));
    let recognized_failure = has_recognized_failure(stdout) || has_recognized_failure(stderr);
    if exit_code != 0 && !stderr.is_empty() && !recognized_failure {
        return Ok(StreamFilterDecision::Unchanged);
    }
    if matches!(command, b"docker" | b"docker-compose")
        && exit_code == 0
        && docker::route(command, argv)
        && (stdout.is_empty() != stderr.is_empty())
    {
        if let Some(compact) = docker::compact(if stdout.is_empty() { stderr } else { stdout }) {
            return Ok(StreamFilterDecision::Applied(StreamFilterOutput::new(
                if stdout.is_empty() {
                    Vec::new()
                } else {
                    compact.clone()
                },
                if stderr.is_empty() {
                    Vec::new()
                } else {
                    compact
                },
                EvidenceClass::PotentiallyLossy,
            )));
        }
    }
    if exit_code == 0 {
        if command == b"vite"
            && catalog_routes::vite_route(argv)
            && catalog_routes::matches_vite(stdout, stderr)
        {
            return Ok(StreamFilterDecision::compact_single_stream(
                stdout,
                stderr,
                EvidenceClass::PotentiallyLossy,
                compact_build_output,
            ));
        }
        if command == b"esbuild"
            && catalog_routes::esbuild_route(argv)
            && catalog_routes::matches_esbuild(stdout, stderr)
        {
            return Ok(StreamFilterDecision::compact_single_stream(
                stdout,
                stderr,
                EvidenceClass::PotentiallyLossy,
                catalog_routes::compact_esbuild,
            ));
        }
        if command == b"cmake"
            && catalog_routes::cmake_route(argv)
            && catalog_routes::matches_cmake(stdout, stderr)
        {
            return Ok(StreamFilterDecision::compact_single_stream(
                stdout,
                stderr,
                EvidenceClass::PotentiallyLossy,
                catalog_routes::compact_cmake,
            ));
        }
    }
    let generic_build_route = matches!(command, b"make" | b"ninja")
        || command == b"cargo" && matches!(arg1, b"build" | b"check" | b"clippy")
        || command == b"go" && arg1 == b"build"
        || command == b"zig" && arg1 == b"build";
    if generic_build_route && matches_build_compact(stdout, stderr) {
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_build,
        ));
    }

    let js_build_route = matches!(command, b"npm" | b"pnpm" | b"yarn" | b"bun")
        && (arg1 == b"build"
            || arg1 == b"run" && argv.get(2).copied().unwrap_or_default() == b"build");
    let frontend_build_route = command == b"webpack"
        || command == b"turbo"
        || command == b"next" && arg1 == b"build"
        || js_build_route;
    if frontend_build_route && (matches_build_output(stdout) || matches_build_output(stderr)) {
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_build_output,
        ));
    }

    if command == b"dotnet" && matches!(arg1, b"build" | b"test" | b"format" | b"restore") {
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_dotnet,
        ));
    }
    if matches!(command, b"gradle" | b"gradlew")
        && (matches_gradle(stdout) || matches_gradle(stderr))
    {
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_gradle,
        ));
    }
    if matches!(command, b"mvn" | b"mvnw") && (matches_maven(stdout) || matches_maven(stderr)) {
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_maven,
        ));
    }
    if matches!(command, b"swift" | b"xcodebuild") {
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_apple_build,
        ));
    }
    if runner_package_prelude {
        return Ok(StreamFilterDecision::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_package_tool,
        ));
    }

    Ok(StreamFilterDecision::Unchanged)
}

pub(crate) fn has_package_prelude(input: &[u8]) -> bool {
    input.split(|byte| *byte == b'\n').any(|raw| {
        let line = trim_ascii_end(trim_ascii_start(raw));
        [
            b"Installed ".as_slice(),
            b"Resolved ",
            b"Prepared ",
            b"Downloaded ",
        ]
        .iter()
        .any(|prefix| {
            line.strip_prefix(*prefix)
                .is_some_and(|rest| find_subslice(rest, b" package").is_some())
        })
    })
}

mod apple;
mod catalog_routes;
mod docker;
mod dotnet;
mod exact;
mod frontend;
mod java;
mod native;

use apple::{compact_apple_build, compact_package_tool, matches_gradle};
use dotnet::{compact_dotnet, compact_evidence, has_recognized_failure, matches_build_output};
use exact::trim_ascii_start;
use frontend::{compact_build_output, matches_build_compact};
use java::{compact_gradle, compact_maven, matches_maven};
use native::compact_build;
