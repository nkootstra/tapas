use super::{
    EvidenceClass, FilterError, StreamFilterOutput, append_line, byte_after_lines,
    command_basename, contains_ignore_ascii_case, find_subslice,
    trim_ascii_end_space as trim_ascii_end,
};

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    argv.first()
        .copied()
        .map(command_basename)
        .is_some_and(|command| {
            matches!(
                command,
                b"make"
                    | b"ninja"
                    | b"cargo"
                    | b"go"
                    | b"zig"
                    | b"npm"
                    | b"pnpm"
                    | b"yarn"
                    | b"bun"
                    | b"webpack"
                    | b"turbo"
                    | b"next"
                    | b"dotnet"
                    | b"gradle"
                    | b"gradlew"
                    | b"mvn"
                    | b"mvnw"
                    | b"swift"
                    | b"xcodebuild"
                    | b"uv"
                    | b"uvx"
                    | b"poetry"
                    | b"npx"
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
    if lossless || crate::invocation_policy::requests_passthrough(argv) {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }

    let command = command_basename(argv[0]);
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let runner_package_prelude = matches!(command, b"uv" | b"uvx" | b"poetry" | b"pnpm" | b"npx")
        && (has_package_prelude(stdout) || has_package_prelude(stderr));
    let recognized_failure = has_recognized_failure(stdout) || has_recognized_failure(stderr);
    if exit_code != 0 && !stderr.is_empty() && !recognized_failure {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }
    let generic_build_route = matches!(command, b"make" | b"ninja")
        || command == b"cargo" && matches!(arg1, b"build" | b"check" | b"clippy")
        || command == b"go" && arg1 == b"build"
        || command == b"zig" && arg1 == b"build";
    if generic_build_route && matches_build_compact(stdout, stderr) {
        return Ok(StreamFilterOutput::compact_single_stream(
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
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_build_output,
        ));
    }

    if command == b"dotnet" && matches!(arg1, b"build" | b"test" | b"format" | b"restore") {
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_dotnet,
        ));
    }
    if matches!(command, b"gradle" | b"gradlew")
        && (matches_gradle(stdout) || matches_gradle(stderr))
    {
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_gradle,
        ));
    }
    if matches!(command, b"mvn" | b"mvnw") && (matches_maven(stdout) || matches_maven(stderr)) {
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_maven,
        ));
    }
    if matches!(command, b"swift" | b"xcodebuild") {
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_apple_build,
        ));
    }
    if matches!(command, b"uv" | b"uvx") || runner_package_prelude {
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            compact_evidence(exit_code),
            compact_package_tool,
        ));
    }

    Ok(StreamFilterOutput::passthrough(stdout, stderr))
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
