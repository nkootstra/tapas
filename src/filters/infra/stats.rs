pub(super) fn route(command: &[u8], argv: &[&[u8]]) -> bool {
    let stats_index = if command == b"docker" && argv.get(1) == Some(&b"stats".as_slice()) {
        1
    } else if command == b"docker"
        && argv.get(1) == Some(&b"compose".as_slice())
        && argv.get(2) == Some(&b"stats".as_slice())
    {
        2
    } else if command == b"docker-compose" && argv.get(1) == Some(&b"stats".as_slice()) {
        1
    } else {
        return false;
    };
    before_terminator(argv)
        .iter()
        .skip(stats_index + 1)
        .any(|argument| {
            *argument == b"--no-stream"
                || argument
                    .strip_prefix(b"--no-stream=")
                    .is_some_and(|value| matches!(value, b"true" | b"1" | b"yes"))
        })
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
        let fields = line
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .collect::<Vec<_>>();
        if fields.len() < offset + 6 || fields[offset + 3] != b"/" {
            return None;
        }
        output.extend_from_slice(fields[offset]);
        output.push(b' ');
        output.extend_from_slice(fields[offset + 1]);
        output.push(b' ');
        output.extend_from_slice(fields[offset + 2]);
        output.push(b'/');
        output.extend_from_slice(fields[offset + 4]);
        output.push(b' ');
        output.extend_from_slice(fields[offset + 5]);
        output.push(b'\n');
    }
    (!output.is_empty()).then_some(output)
}

fn before_terminator<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    let end = argv
        .iter()
        .position(|argument| *argument == b"--")
        .unwrap_or(argv.len());
    &argv[..end]
}
