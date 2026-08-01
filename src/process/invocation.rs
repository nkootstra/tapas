use std::ffi::OsString;

const QUERY_SHORT_FLAG_COMMANDS: &[&[u8]] = &[
    b"pytest",
    b"ruff",
    b"mypy",
    b"prettier",
    b"uv",
    b"uvx",
    b"poetry",
    b"pnpm",
    b"npx",
    b"jq",
];

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

pub fn requests_exact_output(argv: &[OsString]) -> bool {
    let Some(command) = argv.first().map(basename) else {
        return false;
    };
    match command {
        b"find" => argv[1..].iter().any(|argument| {
            is_any(
                bytes(argument),
                &[
                    b"-ls",
                    b"-fls",
                    b"-printf",
                    b"-fprintf",
                    b"-print0",
                    b"-fprint0",
                    b"-exec",
                    b"-execdir",
                    b"-ok",
                    b"-okdir",
                    b"-delete",
                    b"-D",
                ],
            )
        }),
        b"tree" => tree_requests_exact(argv),
        b"ls" => ls_requests_exact(argv),
        b"git" => has_format_or_pretty(argv),
        _ => false,
    }
}

pub fn is_raw_curl(argv: &[OsString]) -> bool {
    argv.first()
        .is_some_and(|command| basename(command) == b"curl")
        && !argv.iter().any(|argument| {
            let argument = bytes(argument);
            argument == b"--verbose"
                || (argument.starts_with(b"-")
                    && !argument.starts_with(b"--")
                    && argument[1..].contains(&b'v'))
        })
}

fn exact_output_reason(command: &[u8], argv: &[OsString]) -> Option<PassthroughReason> {
    if is_query(command, argv) {
        Some(PassthroughReason::Query)
    } else if wants_machine_output(command, argv) {
        Some(PassthroughReason::MachineOutput)
    } else {
        None
    }
}

fn is_query(command: &[u8], argv: &[OsString]) -> bool {
    for argument in &argv[1..] {
        let argument = bytes(argument);
        if argument == b"--" {
            break;
        }
        if is_any(argument, &[b"--help", b"--version"])
            || (argument == b"-h" && is_any(command, QUERY_SHORT_FLAG_COMMANDS))
        {
            return true;
        }
    }
    if argv
        .get(1)
        .is_some_and(|arg| is_any(bytes(arg), &[b"help", b"version"]))
    {
        return true;
    }
    if has_any_arg(argv, &[b"-V"]) && is_any(command, QUERY_SHORT_FLAG_COMMANDS) {
        return true;
    }
    command == b"pytest"
        && has_any_arg(
            argv,
            &[
                b"--collect-only",
                b"--co",
                b"--fixtures",
                b"--fixtures-per-test",
                b"--markers",
                b"--trace-config",
            ],
        )
        || command == b"ruff"
            && argv.get(1).is_some_and(|arg| {
                is_any(
                    bytes(arg),
                    &[b"rule", b"config", b"linter", b"help", b"version"],
                )
            })
        || command == b"prettier"
            && has_any_arg(
                argv,
                &[b"--support-info", b"--find-config-path", b"--file-info"],
            )
        || command == b"tsc" && has_any_arg(argv, &[b"--showConfig", b"--listFilesOnly"])
}

fn wants_machine_output(command: &[u8], argv: &[OsString]) -> bool {
    match command {
        b"rg" => {
            has_any_arg(
                argv,
                &[
                    b"--json",
                    b"--vimgrep",
                    b"-c",
                    b"--count",
                    b"--count-matches",
                    b"-l",
                    b"--files-with-matches",
                    b"--files-without-match",
                    b"--files",
                    b"--type-list",
                    b"-0",
                    b"--null",
                    b"--null-data",
                    b"-o",
                    b"--only-matching",
                    b"--passthru",
                    b"--stats",
                ],
            ) || has_option(argv, b"--replace", b"-r", true)
        }
        b"kubectl" => {
            has_option(argv, b"--output", b"-o", true)
                || has_option(argv, b"--template", b"", false)
                || has_option(argv, b"--label-columns", b"-L", true)
                || has_option(argv, b"--sort-by", b"", false)
                || has_any_arg(
                    argv,
                    &[
                        b"--raw",
                        b"--no-headers",
                        b"--show-labels",
                        b"--output-watch-events",
                        b"--timestamps",
                        b"--prefix",
                    ],
                )
        }
        b"docker" | b"docker-compose" => {
            has_option(argv, b"--format", b"", false)
                || has_any_arg(argv, &[b"-q", b"--quiet", b"--no-trunc"])
                || command == b"docker-compose" && equals_at(argv, 1, b"config")
                || command == b"docker" && equals_at(argv, 1, b"inspect")
                || command == b"docker"
                    && equals_at(argv, 2, b"inspect")
                    && argv.get(1).is_some_and(|object| {
                        is_any(
                            bytes(object),
                            &[
                                b"container",
                                b"image",
                                b"network",
                                b"node",
                                b"plugin",
                                b"secret",
                                b"service",
                                b"volume",
                                b"manifest",
                                b"context",
                            ],
                        )
                    })
                || command == b"docker"
                    && equals_at(argv, 1, b"compose")
                    && equals_at(argv, 2, b"config")
        }
        b"aws" => {
            has_option(argv, b"--output", b"", false)
                || has_option(argv, b"--query", b"", false)
                || has_option(argv, b"--cli-binary-format", b"", false)
                || has_option(argv, b"--generate-cli-skeleton", b"", false)
        }
        b"jq" => true,
        b"ps" => {
            has_option(argv, b"--format", b"-o", true)
                || has_option(argv, b"", b"-O", true)
                || has_option(argv, b"--cols", b"", false)
                || has_option(argv, b"--columns", b"", false)
                || has_option(argv, b"--width", b"", false)
                || has_any_arg(argv, &[b"--headers", b"--no-headers", b"-w", b"-ww"])
        }
        b"psql" => {
            has_any_arg(
                argv,
                &[
                    b"-A",
                    b"--no-align",
                    b"-t",
                    b"--tuples-only",
                    b"-z",
                    b"--field-separator-zero",
                    b"-0",
                    b"--record-separator-zero",
                    b"--csv",
                    b"-H",
                    b"--html",
                    b"-x",
                    b"--expanded",
                ],
            ) || short_bundle_contains(argv, b"Atz0Hx")
                || has_option(argv, b"--field-separator", b"-F", true)
                || has_option(argv, b"--record-separator", b"-R", true)
                || has_option(argv, b"--pset", b"-P", true)
        }
        b"systemctl" => {
            argv.get(1).is_some_and(|subcommand| {
                is_any(
                    bytes(subcommand),
                    &[b"show", b"is-active", b"is-enabled", b"is-failed"],
                )
            }) || has_option(argv, b"--property", b"-p", true)
                || has_any_arg(
                    argv,
                    &[
                        b"--value",
                        b"--no-legend",
                        b"--plain",
                        b"--full",
                        b"--show-types",
                    ],
                )
                || has_option(argv, b"--output", b"-o", true)
        }
        _ => false,
    }
}

fn is_follow_logs(command: &[u8], argv: &[OsString]) -> bool {
    if !has_follow_arg(argv) {
        return false;
    }
    command == b"docker"
        && (equals_at(argv, 1, b"logs")
            || equals_at(argv, 1, b"compose") && equals_at(argv, 2, b"logs"))
        || command == b"docker-compose" && equals_at(argv, 1, b"logs")
        || command == b"kubectl" && equals_at(argv, 1, b"logs")
        || is_any(command, &[b"tail", b"journalctl"])
}

fn has_follow_arg(argv: &[OsString]) -> bool {
    argv.iter().any(|argument| {
        let argument = bytes(argument);
        if is_any(argument, &[b"--follow", b"-f"]) {
            return true;
        }
        let Some(value) = argument.strip_prefix(b"--follow=") else {
            return false;
        };
        !is_any_ascii_case(value, &[b"0", b"false", b"no"])
    })
}

fn tree_requests_exact(argv: &[OsString]) -> bool {
    for argument in &argv[1..] {
        let argument = bytes(argument);
        if argument == b"--" {
            break;
        }
        if argument.len() >= 2
            && argument[0] == b'-'
            && !is_any(
                argument,
                &[
                    b"-a",
                    b"-d",
                    b"-L",
                    b"-I",
                    b"-P",
                    b"--dirsfirst",
                    b"--noreport",
                    b"--prune",
                ],
            )
        {
            return true;
        }
    }
    false
}

fn ls_requests_exact(argv: &[OsString]) -> bool {
    let mut options = true;
    for argument in &argv[1..] {
        let argument = bytes(argument);
        if !options {
            continue;
        }
        if argument == b"--" {
            options = false;
            continue;
        }
        if argument.len() < 2 || argument[0] != b'-' {
            continue;
        }
        if argument[1] == b'-' {
            if is_any(
                argument,
                &[
                    b"--all",
                    b"--almost-all",
                    b"--directory",
                    b"--recursive",
                    b"--classify",
                ],
            ) || argument.starts_with(b"--classify=")
                || argument.starts_with(b"--indicator-style=")
                || argument.strip_prefix(b"--format=").is_some_and(|format| {
                    is_any(
                        format,
                        &[
                            b"across",
                            b"commas",
                            b"horizontal",
                            b"single-column",
                            b"vertical",
                        ],
                    )
                })
            {
                continue;
            }
            return true;
        }
        if argument[1..]
            .iter()
            .any(|flag| !b"1ACFRadmpx".contains(flag))
        {
            return true;
        }
    }
    false
}

fn has_format_or_pretty(argv: &[OsString]) -> bool {
    argv.iter().any(|argument| {
        let argument = bytes(argument);
        is_any(argument, &[b"--format", b"--pretty"])
            || argument.starts_with(b"--format=")
            || argument.starts_with(b"--pretty=")
    })
}

const UV_VALUE: &[&[u8]] = &[
    b"--project",
    b"--directory",
    b"--python",
    b"--package",
    b"--with",
    b"--with-editable",
    b"--with-requirements",
    b"--env-file",
    b"--group",
    b"--extra",
];
const UVX_VALUE: &[&[u8]] = &[
    b"--project",
    b"--directory",
    b"--python",
    b"--package",
    b"--with",
    b"--with-editable",
    b"--with-requirements",
    b"--env-file",
    b"--group",
    b"--extra",
    b"--from",
];
const UV_BOOLEAN: &[&[u8]] = &[
    b"--isolated",
    b"--active",
    b"--no-sync",
    b"--locked",
    b"--frozen",
    b"--no-project",
    b"--all-extras",
    b"--no-dev",
    b"--no-progress",
    b"--offline",
];

fn unwrap_direct<'a>(
    argv: &'a [OsString],
    start: usize,
    values: &[&[u8]],
    booleans: &[&[u8]],
    opaque: &[&[u8]],
) -> Option<&'a [OsString]> {
    let index = scan_options(argv, start, values, booleans, opaque, None)?;
    tool_slice(argv, index)
}

fn unwrap_subcommand<'a>(
    argv: &'a [OsString],
    start: usize,
    subcommand: &[u8],
    values: &[&[u8]],
    booleans: &[&[u8]],
) -> Option<&'a [OsString]> {
    let index = scan_options(argv, start, values, booleans, &[], Some(subcommand))?;
    if !equals_at(argv, index, subcommand) {
        return None;
    }
    tool_slice(argv, index + 1)
}

fn scan_options(
    argv: &[OsString],
    mut index: usize,
    values: &[&[u8]],
    booleans: &[&[u8]],
    opaque: &[&[u8]],
    stop: Option<&[u8]>,
) -> Option<usize> {
    while index < argv.len() {
        let argument = bytes(&argv[index]);
        if stop == Some(argument) {
            return Some(index);
        }
        if argument == b"--" {
            return stop.is_none().then_some(index + 1);
        }
        if argument.is_empty() || argument[0] != b'-' {
            return stop.is_none().then_some(index);
        }
        if option_match(argument, opaque) != OptionMatch::None {
            return None;
        }
        if matches_boolean(argument, booleans) {
            index += 1;
            continue;
        }
        match option_match(argument, values) {
            OptionMatch::Inline => index += 1,
            OptionMatch::Separate if index + 1 < argv.len() => index += 2,
            _ => return None,
        }
    }
    None
}

fn tool_slice(argv: &[OsString], mut index: usize) -> Option<&[OsString]> {
    if equals_at(argv, index, b"--") {
        index += 1;
    }
    argv.get(index)
        .is_some_and(|argument| !bytes(argument).is_empty() && !bytes(argument).starts_with(b"-"))
        .then_some(&argv[index..])
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OptionMatch {
    None,
    Separate,
    Inline,
}

fn option_match(argument: &[u8], options: &[&[u8]]) -> OptionMatch {
    for option in options {
        if argument == *option {
            return OptionMatch::Separate;
        }
        if option.len() > 2
            && option.starts_with(b"--")
            && argument
                .strip_prefix(*option)
                .is_some_and(|rest| rest.starts_with(b"="))
            || option.len() == 2
                && option.starts_with(b"-")
                && argument.len() > 2
                && argument.starts_with(option)
        {
            return OptionMatch::Inline;
        }
    }
    OptionMatch::None
}

fn matches_boolean(argument: &[u8], options: &[&[u8]]) -> bool {
    match option_match(argument, options) {
        OptionMatch::None => false,
        OptionMatch::Separate => true,
        OptionMatch::Inline => {
            argument.len() > 2
                && argument[0] == b'-'
                && argument[1] != b'-'
                && argument[1..].iter().all(|flag| {
                    options
                        .iter()
                        .any(|option| option.len() == 2 && option[1] == *flag)
                })
        }
    }
}

fn has_option(argv: &[OsString], long: &[u8], short: &[u8], joined_short: bool) -> bool {
    argv.iter()
        .take_while(|arg| bytes(arg) != b"--")
        .any(|argument| {
            let argument = bytes(argument);
            !long.is_empty()
                && (argument == long
                    || argument
                        .strip_prefix(long)
                        .is_some_and(|rest| rest.starts_with(b"=")))
                || !short.is_empty()
                    && (argument == short
                        || joined_short
                            && argument.starts_with(short)
                            && argument.len() > short.len())
        })
}

fn short_bundle_contains(argv: &[OsString], needles: &[u8]) -> bool {
    argv.iter()
        .take_while(|arg| bytes(arg) != b"--")
        .any(|argument| {
            let argument = bytes(argument);
            argument.len() >= 3
                && argument[0] == b'-'
                && argument[1] != b'-'
                && argument[1..].iter().any(|byte| needles.contains(byte))
        })
}

fn has_any_arg(argv: &[OsString], needles: &[&[u8]]) -> bool {
    argv.iter()
        .take_while(|argument| bytes(argument) != b"--")
        .any(|argument| is_any(bytes(argument), needles))
}

fn has_arg(argv: &[OsString], needle: &[u8]) -> bool {
    argv.iter().any(|argument| bytes(argument) == needle)
}

fn equals_at(argv: &[OsString], index: usize, expected: &[u8]) -> bool {
    argv.get(index)
        .is_some_and(|argument| bytes(argument) == expected)
}

fn is_any(value: &[u8], options: &[&[u8]]) -> bool {
    options.contains(&value)
}

fn is_any_ascii_case(value: &[u8], options: &[&[u8]]) -> bool {
    options
        .iter()
        .any(|option| value.eq_ignore_ascii_case(option))
}

fn basename(argument: &OsString) -> &[u8] {
    crate::catalog::command_basename(argument.as_os_str())
        .unwrap_or(argument.as_os_str())
        .as_encoded_bytes()
}

fn bytes(argument: &OsString) -> &[u8] {
    argument.as_encoded_bytes()
}
