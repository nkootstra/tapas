use super::*;

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

pub(super) fn exact_output_reason(command: &[u8], argv: &[OsString]) -> Option<PassthroughReason> {
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

pub(super) fn is_follow_logs(command: &[u8], argv: &[OsString]) -> bool {
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
