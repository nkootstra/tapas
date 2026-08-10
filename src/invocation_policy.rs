const QUERY_SHORT_FLAG_COMMANDS: &[&[u8]] = &[
    b"make",
    b"ninja",
    b"cargo",
    b"go",
    b"zig",
    b"npm",
    b"pnpm",
    b"yarn",
    b"bun",
    b"webpack",
    b"turbo",
    b"next",
    b"dotnet",
    b"gradle",
    b"gradlew",
    b"mvn",
    b"mvnw",
    b"swift",
    b"xcodebuild",
    b"uv",
    b"uvx",
    b"vite",
    b"esbuild",
    b"cmake",
    b"ctest",
    b"playwright",
    b"poetry",
    b"npx",
    b"composer",
    b"pip",
    b"pip3",
    b"mypy",
    b"ruff",
    b"eslint",
    b"biome",
    b"pre-commit",
    b"prettier",
    b"terraform",
    b"tofu",
    b"curl",
    b"docker",
    b"docker-compose",
    b"kubectl",
    b"gh",
    b"acli",
    b"helm",
    b"pytest",
    b"jq",
];

pub(crate) fn requests_passthrough(argv: &[&[u8]]) -> bool {
    requests_query(argv) || requests_machine_output(argv) || requests_exact_output(argv)
}

pub(crate) fn requests_query(argv: &[&[u8]]) -> bool {
    let Some(command) = command(argv) else {
        return false;
    };
    let arguments = options(argv);
    if arguments.iter().any(|argument| {
        matches!(*argument, b"--help" | b"--version")
            || matches!(*argument, b"-h" | b"-V") && QUERY_SHORT_FLAG_COMMANDS.contains(&command)
    }) || matches!(argv.get(1), Some(&b"help") | Some(&b"version"))
    {
        return true;
    }

    command == b"pytest"
        && has_any(
            arguments,
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
            && argv.get(1).is_some_and(|argument| {
                matches!(
                    *argument,
                    b"rule" | b"config" | b"linter" | b"help" | b"version"
                )
            })
        || command == b"prettier"
            && has_any(
                arguments,
                &[b"--support-info", b"--find-config-path", b"--file-info"],
            )
        || command == b"tsc" && has_any(arguments, &[b"--showConfig", b"--listFilesOnly"])
        || is_diagnostics(command)
            && arguments.iter().any(|argument| {
                long_option(argument, b"--format")
                    || long_option(argument, b"--output")
                    || long_option(argument, b"--json")
            })
        || command == b"dotnet" && arguments.iter().any(|argument| is_dotnet_query(argument))
}

pub(crate) fn requests_machine_output(argv: &[&[u8]]) -> bool {
    let Some(command) = command(argv) else {
        return false;
    };
    let arguments = options(argv);
    match command {
        b"rg" => {
            has_any(
                arguments,
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
            ) || has_option(arguments, b"--replace", b"-r", true)
        }
        b"kubectl" => {
            has_option(arguments, b"--output", b"-o", true)
                || has_option(arguments, b"--template", b"", false)
                || has_option(arguments, b"--label-columns", b"-L", true)
                || has_option(arguments, b"--sort-by", b"", false)
                || has_any(
                    arguments,
                    &[
                        b"--raw",
                        b"--no-headers",
                        b"--show-labels",
                        b"--output-watch-events",
                        b"--timestamps",
                        b"--prefix",
                        b"--json",
                        b"--jq",
                    ],
                )
        }
        b"helm" => {
            has_option(arguments, b"--output", b"-o", true)
                || argv.get(1).is_some_and(|subcommand| *subcommand == b"get")
        }
        b"docker" | b"docker-compose" => {
            has_option(arguments, b"--format", b"", false)
                || arguments
                    .iter()
                    .any(|argument| *argument == b"--progress=rawjson")
                || arguments
                    .windows(2)
                    .any(|pair| pair[0] == b"--progress" && pair[1] == b"rawjson")
                || has_any(arguments, &[b"-q", b"--quiet", b"--no-trunc"])
                || command == b"docker-compose" && equals_at(argv, 1, b"config")
                || command == b"docker" && equals_at(argv, 1, b"inspect")
                || command == b"docker"
                    && equals_at(argv, 2, b"inspect")
                    && argv.get(1).is_some_and(|object| {
                        [
                            b"container".as_slice(),
                            b"image",
                            b"network",
                            b"node",
                            b"plugin",
                            b"secret",
                            b"service",
                            b"volume",
                            b"manifest",
                            b"context",
                        ]
                        .contains(object)
                    })
                || command == b"docker"
                    && equals_at(argv, 1, b"compose")
                    && equals_at(argv, 2, b"config")
        }
        b"aws" => [
            b"--output".as_slice(),
            b"--query",
            b"--cli-binary-format",
            b"--generate-cli-skeleton",
        ]
        .iter()
        .any(|option| has_option(arguments, option, b"", false)),
        b"jq" => true,
        b"ps" => {
            has_option(arguments, b"--format", b"-o", true)
                || has_option(arguments, b"", b"-O", true)
                || has_option(arguments, b"--cols", b"", false)
                || has_option(arguments, b"--columns", b"", false)
                || has_option(arguments, b"--width", b"", false)
                || has_any(arguments, &[b"--headers", b"--no-headers", b"-w", b"-ww"])
        }
        b"psql" => {
            has_any(
                arguments,
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
            ) || short_bundle_contains(arguments, b"Atz0Hx")
                || has_option(arguments, b"--field-separator", b"-F", true)
                || has_option(arguments, b"--record-separator", b"-R", true)
                || has_option(arguments, b"--pset", b"-P", true)
        }
        b"systemctl" => {
            argv.get(1).is_some_and(|subcommand| {
                matches!(
                    *subcommand,
                    b"show" | b"is-active" | b"is-enabled" | b"is-failed"
                )
            }) || has_option(arguments, b"--property", b"-p", true)
                || has_any(
                    arguments,
                    &[
                        b"--value",
                        b"--no-legend",
                        b"--plain",
                        b"--full",
                        b"--show-types",
                    ],
                )
                || has_option(arguments, b"--output", b"-o", true)
        }
        b"npm" | b"pnpm" | b"yarn" | b"bun" | b"composer" | b"pip" | b"pip3" | b"uv" | b"uvx" => {
            arguments.iter().any(|argument| {
                matches!(
                    *argument,
                    b"--json"
                        | b"--ndjson"
                        | b"--parseable"
                        | b"--porcelain"
                        | b"--json-stream"
                        | b"--format"
                        | b"--reporter"
                ) || long_option(argument, b"--json")
                    || long_option(argument, b"--format")
                    || long_option(argument, b"--reporter")
            })
        }
        // `gh --jq` and `gh --template` emit caller-shaped output that must
        // stay byte-exact; plain `--json` is compacted by the gh filter.
        b"gh" => {
            has_any(arguments, &[b"--jq", b"--template"])
                || has_option(arguments, b"--jq", b"", true)
                || has_option(arguments, b"--template", b"", true)
        }
        b"sqlite3" => {
            has_any(
                arguments,
                &[
                    b"-csv",
                    b"-html",
                    b"-json",
                    b"-line",
                    b"-list",
                    b"-markdown",
                    b"-noheader",
                    b"-nullvalue",
                    b"-quote",
                    b"-separator",
                    b"-tabs",
                ],
            ) || has_option(arguments, b"-separator", b"", true)
                || has_option(arguments, b"-nullvalue", b"", true)
        }
        _ => false,
    }
}

pub(crate) fn requests_exact_output(argv: &[&[u8]]) -> bool {
    let Some(command) = command(argv) else {
        return false;
    };
    let arguments = options(argv);
    match command {
        b"pip" | b"pip3" => argv.get(1).is_some_and(|subcommand| {
            matches!(*subcommand, b"freeze" | b"show" | b"check" | b"inspect")
        }),
        b"ctest" => options(argv).iter().any(|argument| {
            matches!(
                *argument,
                b"-N"
                    | b"--show-only"
                    | b"--print-labels"
                    | b"--output-junit"
                    | b"-D"
                    | b"-M"
                    | b"-T"
                    | b"-S"
            ) || argument.starts_with(b"--show-only=")
                || argument.starts_with(b"--output-junit=")
        }),
        b"playwright" => {
            has_any(options(argv), &[b"--list"])
                || options(argv).iter().any(|argument| {
                    argument
                        .strip_prefix(b"--reporter=")
                        .is_some_and(|reporter| !matches!(reporter, b"list" | b"line" | b"dot"))
                })
        }
        b"cat" | b"bat" | b"batcat" => arguments.iter().any(|argument| argument.starts_with(b"-")),
        b"grep" => arguments.iter().any(|argument| {
            matches!(
                *argument,
                b"-c"
                    | b"--count"
                    | b"-l"
                    | b"--files-with-matches"
                    | b"-L"
                    | b"--files-without-match"
                    | b"-o"
                    | b"--only-matching"
                    | b"-q"
                    | b"--quiet"
                    | b"-b"
                    | b"--byte-offset"
                    | b"-z"
                    | b"--null-data"
                    | b"-Z"
                    | b"--null"
                    | b"-a"
                    | b"--text"
                    | b"-I"
                    | b"--binary-files"
            ) || argument.starts_with(b"-A")
                || argument.starts_with(b"-B")
                || argument.starts_with(b"-C")
                || argument.starts_with(b"--after-context")
                || argument.starts_with(b"--before-context")
                || argument.starts_with(b"--context")
        }),
        b"find" => has_any(
            arguments,
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
        ),
        b"tree" => arguments.iter().any(|argument| {
            argument.len() >= 2
                && argument[0] == b'-'
                && !matches!(
                    *argument,
                    b"-a"
                        | b"-d"
                        | b"-L"
                        | b"-I"
                        | b"-P"
                        | b"--dirsfirst"
                        | b"--noreport"
                        | b"--prune"
                )
        }),
        b"ls" => ls_requests_exact(arguments),
        b"git" => arguments.iter().any(|argument| {
            matches!(*argument, b"--format" | b"--pretty")
                || argument.starts_with(b"--format=")
                || argument.starts_with(b"--pretty=")
        }),
        _ => false,
    }
}

fn options<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    let arguments = argv.get(1..).unwrap_or_default();
    &arguments[..arguments
        .iter()
        .position(|argument| *argument == b"--")
        .unwrap_or(arguments.len())]
}

fn command<'a>(argv: &'a [&'a [u8]]) -> Option<&'a [u8]> {
    argv.first()
        .map(|command| crate::catalog::command_basename_bytes(command))
}

fn has_any(arguments: &[&[u8]], needles: &[&[u8]]) -> bool {
    arguments.iter().any(|argument| needles.contains(argument))
}

fn has_option(arguments: &[&[u8]], long: &[u8], short: &[u8], joined_short: bool) -> bool {
    arguments.iter().any(|argument| {
        !long.is_empty() && long_option(argument, long)
            || !short.is_empty()
                && (*argument == short
                    || joined_short && argument.starts_with(short) && argument.len() > short.len())
    })
}

fn long_option(argument: &[u8], option: &[u8]) -> bool {
    argument == option
        || argument
            .strip_prefix(option)
            .is_some_and(|rest| rest.starts_with(b"="))
}

fn short_bundle_contains(arguments: &[&[u8]], needles: &[u8]) -> bool {
    arguments.iter().any(|argument| {
        argument.len() >= 3
            && argument[0] == b'-'
            && argument[1] != b'-'
            && argument[1..].iter().any(|byte| needles.contains(byte))
    })
}

fn equals_at(argv: &[&[u8]], index: usize, expected: &[u8]) -> bool {
    argv.get(index)
        .is_some_and(|argument| *argument == expected)
}

fn is_diagnostics(command: &[u8]) -> bool {
    matches!(
        command,
        b"mypy"
            | b"ruff"
            | b"eslint"
            | b"biome"
            | b"pre-commit"
            | b"prettier"
            | b"terraform"
            | b"tofu"
    )
}

fn is_dotnet_query(argument: &[u8]) -> bool {
    let rest = if let Some(rest) = argument.strip_prefix(b"--") {
        rest
    } else if matches!(argument.first(), Some(b'-' | b'/')) {
        &argument[1..]
    } else {
        return false;
    };
    [b"getproperty".as_slice(), b"getitem", b"gettargetresult"]
        .iter()
        .any(|query| {
            rest.get(..query.len())
                .is_some_and(|name| name.eq_ignore_ascii_case(query))
                && rest.get(query.len()) == Some(&b':')
        })
}

fn ls_requests_exact(arguments: &[&[u8]]) -> bool {
    for argument in arguments {
        if argument.len() < 2 || argument[0] != b'-' {
            continue;
        }
        if argument[1] == b'-' {
            if matches!(
                *argument,
                b"--all" | b"--almost-all" | b"--directory" | b"--recursive" | b"--classify"
            ) || argument.starts_with(b"--classify=")
                || argument.starts_with(b"--indicator-style=")
                || argument.strip_prefix(b"--format=").is_some_and(|format| {
                    matches!(
                        format,
                        b"across" | b"commas" | b"horizontal" | b"single-column" | b"vertical"
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
