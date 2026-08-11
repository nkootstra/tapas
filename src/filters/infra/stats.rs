use crate::invocation_policy::{
    COMPOSE_BOOLEAN_OPTIONS, COMPOSE_VALUE_OPTIONS, option_consumption,
};

pub(super) fn route(command: &[u8], argv: &[&[u8]]) -> bool {
    let stats_index = if command == b"docker" && argv.get(1) == Some(&b"stats".as_slice()) {
        1
    } else if command == b"docker" && argv.get(1) == Some(&b"compose".as_slice()) {
        let Some(index) = compose_stats_index(argv, 2) else {
            return false;
        };
        index
    } else if command == b"docker-compose" {
        let Some(index) = compose_stats_index(argv, 1) else {
            return false;
        };
        index
    } else {
        return false;
    };
    crate::invocation_policy::options(argv)
        .iter()
        .skip(stats_index)
        .any(|argument| {
            *argument == b"--no-stream"
                || argument
                    .strip_prefix(b"--no-stream=")
                    .is_some_and(|value| matches!(value, b"true" | b"1" | b"yes"))
        })
}

fn compose_stats_index(argv: &[&[u8]], mut index: usize) -> Option<usize> {
    while index < argv.len() {
        let argument = argv[index];
        if argument == b"--" {
            return None;
        }
        if !argument.starts_with(b"-") {
            return (argument == b"stats").then_some(index);
        }
        if COMPOSE_BOOLEAN_OPTIONS.contains(&argument) {
            index += 1;
        } else {
            index += option_consumption(argument, COMPOSE_VALUE_OPTIONS)?;
        }
    }
    None
}

pub(super) fn compact(input: &[u8]) -> Option<Vec<u8>> {
    let mut lines = input.split(|byte| *byte == b'\n');
    let header = lines.next()?;
    let container_id =
        header.starts_with(b"CONTAINER ID") && header.windows(4).any(|part| part == b"CPU ");
    let compose = header.starts_with(b"NAME") && header.windows(4).any(|part| part == b"CPU ");
    if !container_id && !compose {
        return None;
    }
    let offset = usize::from(container_id);
    let mut output = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let mut fields = line
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty());
        if offset == 1 {
            fields.next()?;
        }
        let name = fields.next()?;
        let cpu = fields.next()?;
        let memory = fields.next()?;
        if fields.next()? != b"/" {
            return None;
        }
        let memory_limit = fields.next()?;
        let memory_percent = fields.next()?;
        output.extend_from_slice(name);
        output.push(b' ');
        output.extend_from_slice(cpu);
        output.push(b' ');
        output.extend_from_slice(memory);
        output.push(b'/');
        output.extend_from_slice(memory_limit);
        output.push(b' ');
        output.extend_from_slice(memory_percent);
        output.push(b'\n');
    }
    (!output.is_empty()).then_some(output)
}
