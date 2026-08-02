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
    let Some(command) = argv.first() else {
        return Invocation {
            logical_argv: argv,
            passthrough_reason: None,
        };
    };
    let command = basename(command);
    let mut ambiguous = false;
    let logical = if command == b"uv" && equals_at(argv, 1, b"run") {
        unwrap_direct(argv, 2, UV_VALUE, UV_BOOLEAN, &[]).unwrap_or_else(|| {
            ambiguous = true;
            argv
        })
    } else if command == b"uvx" {
        unwrap_direct(argv, 1, UVX_VALUE, UV_BOOLEAN, &[]).unwrap_or_else(|| {
            ambiguous = true;
            argv
        })
    } else if command == b"poetry" {
        unwrap_subcommand(
            argv,
            1,
            b"run",
            &[b"-C", b"--directory", b"-P", b"--project"],
            &[b"--no-interaction", b"--no-ansi", b"-q", b"--quiet"],
        )
        .unwrap_or_else(|| {
            ambiguous = true;
            argv
        })
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
        .unwrap_or_else(|| {
            ambiguous = true;
            argv
        })
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
        .unwrap_or_else(|| {
            ambiguous = true;
            argv
        })
    } else {
        argv
    };

    if ambiguous {
        return Invocation {
            logical_argv: logical,
            passthrough_reason: exact_output_reason(command, argv)
                .or(Some(PassthroughReason::AmbiguousRunner)),
        };
    }
    Invocation {
        logical_argv: logical,
        passthrough_reason: exact_output_reason(basename(&logical[0]), logical),
    }
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

    let watch_capable = is_any(
        command,
        &[b"jest", b"vitest", b"tsc", b"webpack", b"nodemon"],
    );
    let js_runner = is_any(command, &[b"npm", b"pnpm", b"yarn", b"bun", b"deno"]);
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

mod policy;
mod runners;

use policy::{exact_output_reason, is_follow_logs};
pub use policy::{is_raw_curl, requests_exact_output};
use runners::{
    UV_BOOLEAN, UV_VALUE, UVX_VALUE, basename, bytes, equals_at, has_any_arg, has_arg, is_any,
    unwrap_direct, unwrap_subcommand,
};
