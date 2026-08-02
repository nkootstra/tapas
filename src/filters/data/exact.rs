use super::*;

pub(super) fn gh_wants_data_output(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        matches!(*argument, b"--json" | b"--jq")
            || argument.starts_with(b"--json=")
            || argument.starts_with(b"--jq=")
    })
}

pub(super) fn requests_query(argv: &[&[u8]]) -> bool {
    let command = command_basename(argv[0]);
    for argument in &argv[1..] {
        if *argument == b"--" {
            break;
        }
        if matches!(*argument, b"--help" | b"--version") {
            return true;
        }
        if matches!(*argument, b"-h" | b"-V") && command == b"jq" {
            return true;
        }
    }
    matches!(argv.get(1), Some(&b"help") | Some(&b"version"))
}

pub(super) fn requests_machine_output(command: &[u8], argv: &[&[u8]]) -> bool {
    match command {
        b"aws" => {
            has_long_option(argv, b"--output")
                || has_long_option(argv, b"--query")
                || has_long_option(argv, b"--cli-binary-format")
                || has_long_option(argv, b"--generate-cli-skeleton")
        }
        b"jq" => argv[1..].iter().any(|argument| {
            matches!(
                *argument,
                b"-c"
                    | b"--compact-output"
                    | b"-r"
                    | b"--raw-output"
                    | b"-j"
                    | b"--join-output"
                    | b"--stream"
                    | b"--seq"
            )
        }),
        b"ps" => argv[1..].iter().any(|argument| {
            matches!(
                *argument,
                b"-o"
                    | b"-O"
                    | b"-w"
                    | b"-ww"
                    | b"--headers"
                    | b"--no-headers"
                    | b"--format"
                    | b"--cols"
                    | b"--columns"
                    | b"--width"
            ) || argument.starts_with(b"-o")
                || argument.starts_with(b"-O")
                || argument.starts_with(b"--format=")
                || argument.starts_with(b"--cols=")
                || argument.starts_with(b"--columns=")
                || argument.starts_with(b"--width=")
        }),
        b"psql" => argv[1..].iter().any(|argument| {
            matches!(
                *argument,
                b"-A"
                    | b"--no-align"
                    | b"-t"
                    | b"--tuples-only"
                    | b"-z"
                    | b"--field-separator-zero"
                    | b"-0"
                    | b"--record-separator-zero"
                    | b"--csv"
                    | b"-H"
                    | b"--html"
                    | b"-x"
                    | b"--expanded"
                    | b"-F"
                    | b"--field-separator"
                    | b"-R"
                    | b"--record-separator"
                    | b"-P"
                    | b"--pset"
            ) || argument.starts_with(b"--field-separator=")
                || argument.starts_with(b"--record-separator=")
                || argument.starts_with(b"--pset=")
                || short_bundle_contains(argument, b"Atz0Hx")
        }),
        b"systemctl" => {
            argv.get(1).is_some_and(|subcommand| {
                matches!(
                    *subcommand,
                    b"show" | b"is-active" | b"is-enabled" | b"is-failed"
                )
            }) || argv[1..].iter().any(|argument| {
                matches!(
                    *argument,
                    b"--property"
                        | b"-p"
                        | b"--value"
                        | b"--no-legend"
                        | b"--plain"
                        | b"--full"
                        | b"--show-types"
                        | b"--output"
                        | b"-o"
                ) || argument.starts_with(b"--property=")
                    || argument.starts_with(b"--output=")
                    || argument.starts_with(b"-p") && argument.len() > 2
                    || argument.starts_with(b"-o") && argument.len() > 2
            })
        }
        b"kubectl" => argv[1..].iter().any(|argument| {
            matches!(
                *argument,
                b"--output"
                    | b"-o"
                    | b"--template"
                    | b"--label-columns"
                    | b"-L"
                    | b"--sort-by"
                    | b"--raw"
                    | b"--no-headers"
                    | b"--show-labels"
                    | b"--output-watch-events"
                    | b"--timestamps"
                    | b"--prefix"
            ) || argument.starts_with(b"--output=")
                || argument.starts_with(b"-o") && argument.len() > 2
                || argument.starts_with(b"--template=")
                || argument.starts_with(b"--label-columns=")
                || argument.starts_with(b"-L") && argument.len() > 2
                || argument.starts_with(b"--sort-by=")
        }),
        b"docker" | b"docker-compose" => {
            argv[1..].iter().any(|argument| {
                matches!(*argument, b"--format" | b"-q" | b"--quiet" | b"--no-trunc")
                    || argument.starts_with(b"--format=")
            }) || command == b"docker-compose" && argv.get(1).is_some_and(|arg| *arg == b"config")
                || command == b"docker" && argv.get(1).is_some_and(|arg| *arg == b"inspect")
                || command == b"docker" && argv.get(2).is_some_and(|arg| *arg == b"inspect")
                || command == b"docker"
                    && argv.get(1).is_some_and(|arg| *arg == b"compose")
                    && argv.get(2).is_some_and(|arg| *arg == b"config")
        }
        _ => false,
    }
}

fn has_long_option(argv: &[&[u8]], option: &[u8]) -> bool {
    argv[1..].iter().any(|argument| {
        *argument == option
            || argument
                .strip_prefix(option)
                .is_some_and(|suffix| suffix.starts_with(b"="))
    })
}

fn short_bundle_contains(argument: &[u8], needles: &[u8]) -> bool {
    argument.len() > 2
        && argument[0] == b'-'
        && argument[1] != b'-'
        && argument[1..].iter().any(|byte| needles.contains(byte))
}

pub mod sigil_rle {
    const PREFIX_LEN: usize = 16;
    const SIGIL: u8 = 0x01;

    pub fn encode(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut previous_prefix = &[][..];
        let mut first = true;
        let mut offset = 0usize;
        while offset < input.len() {
            let relative_end = input[offset..].iter().position(|byte| *byte == b'\n');
            let end = relative_end.map_or(input.len(), |index| offset + index);
            let line = &input[offset..end];
            if line.first() == Some(&SIGIL) {
                output.push(SIGIL);
                output.extend_from_slice(line);
            } else {
                let prefix_len = line.len().min(PREFIX_LEN);
                let prefix = &line[..prefix_len];
                let can_elide = !first
                    && prefix_len == PREFIX_LEN
                    && prefix == previous_prefix
                    && (line.len() == PREFIX_LEN || line[PREFIX_LEN] != SIGIL);
                if can_elide {
                    output.push(SIGIL);
                    output.extend_from_slice(&line[PREFIX_LEN..]);
                } else {
                    output.extend_from_slice(line);
                }
            }
            previous_prefix = &line[..line.len().min(PREFIX_LEN)];
            if end < input.len() {
                output.push(b'\n');
                offset = end + 1;
            } else {
                offset = end;
            }
            first = false;
        }
        output
    }

    pub fn decode(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut previous_prefix = [0u8; PREFIX_LEN];
        let mut previous_len = 0usize;
        let mut offset = 0usize;
        while offset < input.len() {
            let relative_end = input[offset..].iter().position(|byte| *byte == b'\n');
            let end = relative_end.map_or(input.len(), |index| offset + index);
            let line = &input[offset..end];
            if line.starts_with(&[SIGIL, SIGIL]) {
                let decoded = &line[1..];
                output.extend_from_slice(decoded);
                previous_len = decoded.len().min(PREFIX_LEN);
                previous_prefix[..previous_len].copy_from_slice(&decoded[..previous_len]);
            } else if line.first() == Some(&SIGIL) {
                output.extend_from_slice(&previous_prefix[..previous_len]);
                output.extend_from_slice(&line[1..]);
            } else {
                output.extend_from_slice(line);
                previous_len = line.len().min(PREFIX_LEN);
                previous_prefix[..previous_len].copy_from_slice(&line[..previous_len]);
            }
            if end < input.len() {
                output.push(b'\n');
                offset = end + 1;
            } else {
                offset = end;
            }
        }
        output
    }
}

pub mod ws_rle {
    use super::FilterError;

    const SIGIL: u8 = 0x01;
    const MIN_RUN: usize = 17;
    const MAX_RUN: usize = 255;

    pub fn encode(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut index = 0usize;
        while index < input.len() {
            if input[index] == SIGIL {
                output.extend_from_slice(&[SIGIL, 0]);
                index += 1;
            } else if input[index] == b' ' {
                let mut after = index;
                while after < input.len() && input[after] == b' ' {
                    after += 1;
                }
                let mut run = after - index;
                if run < MIN_RUN {
                    output.extend_from_slice(&input[index..after]);
                } else {
                    while run > 0 {
                        let chunk = run.min(MAX_RUN);
                        if chunk < MIN_RUN {
                            output.extend(std::iter::repeat_n(b' ', chunk));
                        } else {
                            output.extend_from_slice(&[SIGIL, chunk as u8]);
                        }
                        run -= chunk;
                    }
                }
                index = after;
            } else {
                output.push(input[index]);
                index += 1;
            }
        }
        output
    }

    pub fn decode(input: &[u8]) -> Result<Vec<u8>, FilterError> {
        let mut output = Vec::with_capacity(input.len());
        let mut index = 0usize;
        while index < input.len() {
            if input[index] != SIGIL {
                output.push(input[index]);
                index += 1;
                continue;
            }
            let length = *input.get(index + 1).ok_or(FilterError::InvalidInput)?;
            if length == 0 {
                output.push(SIGIL);
            } else if usize::from(length) >= MIN_RUN {
                output.extend(std::iter::repeat_n(b' ', usize::from(length)));
            } else {
                return Err(FilterError::InvalidInput);
            }
            index += 2;
        }
        Ok(output)
    }
}
