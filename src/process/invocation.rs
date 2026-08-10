use std::ffi::OsString;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PassthroughReason {
    Query,
    MachineOutput,
    AmbiguousRunner,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Invocation<'a> {
    pub logical_argv: &'a [OsString],
    pub passthrough_reason: Option<PassthroughReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamDecision {
    Capture,
    Inherit,
    StreamFilter,
}

pub fn classify(argv: &[OsString]) -> Invocation<'_> {
    if argv.is_empty() {
        return Invocation {
            logical_argv: argv,
            passthrough_reason: None,
        };
    }

    let mut logical = argv;
    for _ in 0..MAX_RUNNER_LAYERS {
        match unwrap_runner(logical) {
            RunnerStep::NotRunner => break,
            RunnerStep::Unwrapped(inner) => logical = inner,
            RunnerStep::Ambiguous => return ambiguous_invocation(argv),
        }
    }
    if !matches!(unwrap_runner(logical), RunnerStep::NotRunner) {
        return ambiguous_invocation(argv);
    }

    Invocation {
        logical_argv: logical,
        passthrough_reason: exact_output_reason(basename(&logical[0]), logical),
    }
}

pub fn is_supported(argv: &[OsString]) -> bool {
    let classified = classify(argv);
    classified.passthrough_reason != Some(PassthroughReason::AmbiguousRunner)
        && classified
            .logical_argv
            .first()
            .is_some_and(|command| crate::catalog::should_auto_wrap(command))
}

const MAX_RUNNER_LAYERS: usize = 4;

enum RunnerStep<'a> {
    NotRunner,
    Unwrapped(&'a [OsString]),
    Ambiguous,
}

fn ambiguous_invocation(argv: &[OsString]) -> Invocation<'_> {
    Invocation {
        logical_argv: argv,
        passthrough_reason: Some(PassthroughReason::AmbiguousRunner),
    }
}

fn unwrap_runner(argv: &[OsString]) -> RunnerStep<'_> {
    let command = basename(&argv[0]);
    let inner = if command == b"uv" && equals_at(argv, 1, b"run") {
        unwrap_direct(argv, 2, UV_VALUE, UV_BOOLEAN, &[])
    } else if command == b"uvx" {
        unwrap_direct(argv, 1, UVX_VALUE, UV_BOOLEAN, &[])
    } else if command == b"poetry" {
        unwrap_subcommand(
            argv,
            1,
            b"run",
            &[b"-C", b"--directory", b"-P", b"--project"],
            &[b"--no-interaction", b"--no-ansi", b"-q", b"--quiet"],
        )
    } else if command == b"pnpm"
        && argv.len() >= 2
        && (has_any_arg(argv, &[b"exec"]) || bytes(&argv[1]).starts_with(b"-"))
    {
        unwrap_subcommand(
            argv,
            1,
            b"exec",
            &[
                b"-C",
                b"--dir",
                b"-F",
                b"--filter",
                b"--workspace-concurrency",
            ],
            &[
                b"-r",
                b"--recursive",
                b"-w",
                b"--workspace-root",
                b"--parallel",
                b"--stream",
                b"--aggregate-output",
                b"--use-stderr",
            ],
        )
    } else if command == b"npx" {
        unwrap_direct(
            argv,
            1,
            &[
                b"-p",
                b"--package",
                b"-w",
                b"--workspace",
                b"--allow-scripts",
            ],
            &[
                b"-y",
                b"--yes",
                b"--no",
                b"--workspaces",
                b"--include-workspace-root",
                b"--strict-allow-scripts",
                b"--dangerously-allow-all-scripts",
            ],
            &[b"-c", b"--call"],
        )
    } else if command == b"bunx" {
        unwrap_direct(
            argv,
            1,
            &[b"--package", b"--cwd"],
            &[
                b"--bun",
                b"--no-install",
                b"--silent",
                b"--help",
                b"--version",
            ],
            &[],
        )
    } else {
        return RunnerStep::NotRunner;
    };
    inner.map_or(RunnerStep::Ambiguous, RunnerStep::Unwrapped)
}

pub fn classify_stream(argv: &[OsString]) -> StreamDecision {
    let Some(command) = argv.first() else {
        return StreamDecision::Capture;
    };
    let command = basename(command);
    if is_follow_logs(command, argv)
        || (command == b"tsc" && has_any_arg(argv, &[b"--watch", b"-w"]))
        || (is_any(command, &[b"jest", b"vitest"])
            && has_any_arg(argv, &[b"--watch", b"--watchAll", b"-w"]))
        || (command == b"gh" && equals_at(argv, 1, b"run") && equals_at(argv, 2, b"watch"))
    {
        return StreamDecision::StreamFilter;
    }

    if inherits_lifecycle(command, argv) {
        return StreamDecision::Inherit;
    }

    let watch_capable = is_any(
        command,
        &[b"jest", b"vitest", b"tsc", b"webpack", b"nodemon"],
    );
    let js_runner = is_any(
        command,
        &[b"npm", b"pnpm", b"yarn", b"bun", b"bunx", b"deno"],
    );
    let dev_server = is_any(command, &[b"vite", b"next", b"nuxt", b"webpack"]);
    if has_any_arg(argv, &[b"--watch", b"--watchAll"]) && (watch_capable || js_runner || dev_server)
        || (has_arg(argv, b"-w") && watch_capable)
    {
        return StreamDecision::Inherit;
    }
    if let Some(subcommand) = argv.get(1).map(bytes) {
        if subcommand == b"watch" && (js_runner || command == b"cargo" || command == b"gh") {
            return StreamDecision::Inherit;
        }
        if command == b"go" && subcommand == b"run" {
            return StreamDecision::Inherit;
        }
        if is_any(subcommand, &[b"dev", b"serve", b"start"]) && (js_runner || dev_server) {
            return StreamDecision::Inherit;
        }
    }
    if command == b"bunx"
        && argv
            .iter()
            .skip(1)
            .map(bytes)
            .any(|argument| is_any(argument, &[b"dev", b"serve", b"start", b"watch"]))
    {
        return StreamDecision::Inherit;
    }
    if argv.len() >= 3
        && js_runner
        && is_any(bytes(&argv[1]), &[b"run", b"exec", b"task"])
        && is_any(bytes(&argv[2]), &[b"dev", b"serve", b"start", b"watch"])
    {
        return StreamDecision::Inherit;
    }
    if is_any(command, &[b"nodemon", b"watchman"]) {
        StreamDecision::Inherit
    } else {
        StreamDecision::Capture
    }
}

fn inherits_lifecycle(command: &[u8], argv: &[OsString]) -> bool {
    if command == b"vite" {
        let subcommand = argv.get(1).map(bytes);
        return subcommand.is_none()
            || subcommand.is_some_and(|value| is_any(value, &[b"dev", b"serve", b"preview"]))
            || subcommand == Some(b"build") && long_option_enabled(argv, b"--watch");
    }
    if command == b"esbuild" {
        return long_option_enabled(argv, b"--watch") || long_option_enabled(argv, b"--serve");
    }
    if command == b"playwright" {
        return has_enabled_option(argv, &[b"--ui", b"--debug", b"--headed"])
            || argv.get(1).map(bytes).is_some_and(|subcommand| {
                is_any(subcommand, &[b"show-report", b"show-trace", b"codegen"])
            });
    }
    if command == b"docker" {
        if equals_at(argv, 1, b"run") {
            return true;
        }
        if equals_at(argv, 1, b"compose") && equals_at(argv, 2, b"up") {
            return !boolean_option_enabled(argv, b"--detach", Some(b'd')).unwrap_or(false);
        }
        if equals_at(argv, 1, b"stats") {
            return !boolean_option_enabled(argv, b"--no-stream", None).unwrap_or(false);
        }
    }
    if command == b"docker-compose" && equals_at(argv, 1, b"up") {
        return !boolean_option_enabled(argv, b"--detach", Some(b'd')).unwrap_or(false);
    }
    if is_any(command, &[b"bat", b"batcat"]) {
        return option_value(argv, b"--paging").is_some_and(|value| value == b"always");
    }
    command == b"ctest" && option_value(argv, b"--repeat").is_some()
}

fn has_enabled_option(argv: &[OsString], options: &[&[u8]]) -> bool {
    options
        .iter()
        .any(|option| long_option_enabled(argv, option))
}

fn long_option_enabled(argv: &[OsString], option: &[u8]) -> bool {
    boolean_option_enabled(argv, option, None).unwrap_or(false)
}

fn boolean_option_enabled(argv: &[OsString], long: &[u8], short: Option<u8>) -> Option<bool> {
    let mut result = None;
    let mut index = 1;
    while index < argv.len() && bytes(&argv[index]) != b"--" {
        let argument = bytes(&argv[index]);
        if argument == long {
            result = Some(
                argv.get(index + 1)
                    .and_then(|value| boolean_value(bytes(value)))
                    .unwrap_or(true),
            );
        } else if let Some(value) = argument
            .strip_prefix(long)
            .and_then(|rest| rest.strip_prefix(b"="))
        {
            result = boolean_value(value).or(Some(true));
        } else if short.is_some_and(|short| {
            argument.starts_with(b"-")
                && !argument.starts_with(b"--")
                && argument[1..].contains(&short)
        }) {
            result = Some(true);
        }
        index += 1;
    }
    result
}

fn boolean_value(value: &[u8]) -> Option<bool> {
    if is_any_ascii_case(value, &[b"true".as_slice(), b"1", b"yes"]) {
        Some(true)
    } else if is_any_ascii_case(value, &[b"false".as_slice(), b"0", b"no"]) {
        Some(false)
    } else {
        None
    }
}

fn option_value<'a>(argv: &'a [OsString], option: &[u8]) -> Option<&'a [u8]> {
    for (index, argument) in argv.iter().enumerate().skip(1) {
        let argument = bytes(argument);
        if argument == b"--" {
            return None;
        }
        if argument == option {
            return argv.get(index + 1).map(bytes);
        }
        if let Some(value) = argument
            .strip_prefix(option)
            .and_then(|rest| rest.strip_prefix(b"="))
        {
            return Some(value);
        }
    }
    None
}

mod policy;
mod runners;

use policy::{exact_output_reason, is_follow_logs};
pub use policy::{is_raw_curl, requests_exact_output};
use runners::{
    UV_BOOLEAN, UV_VALUE, UVX_VALUE, basename, bytes, equals_at, has_any_arg, has_arg, is_any,
    is_any_ascii_case, unwrap_direct, unwrap_subcommand,
};
