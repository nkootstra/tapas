pub(super) fn du_has_summarize(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        matches!(*argument, b"-s" | b"--summarize")
            || (argument.len() >= 2
                && argument[0] == b'-'
                && argument[1] != b'-'
                && argument[1..].contains(&b's'))
    })
}

pub(super) fn apply_wc(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if stdout.is_empty() {
        return stderr.to_vec();
    }
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for raw in stdout.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let line = raw.trim_ascii();
        let mut position = 0;
        let mut counts = 0;
        while position < line.len() && counts < 3 {
            let start = position;
            while line.get(position).is_some_and(u8::is_ascii_digit) {
                position += 1;
            }
            if position == start {
                break;
            }
            if counts > 0 {
                output.push(b' ');
            }
            output.extend_from_slice(&line[start..position]);
            counts += 1;
            while line
                .get(position)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                position += 1;
            }
        }
        if counts == 0 {
            output.extend_from_slice(raw);
        } else if position < line.len() {
            output.push(b' ');
            output.extend_from_slice(&line[position..]);
        }
        output.push(b'\n');
    }
    output.extend_from_slice(stderr);
    output
}

pub(super) fn env_is_listing(argv: &[&[u8]]) -> bool {
    argv[1..].iter().all(|argument| {
        argument.is_empty()
            || argument[0] == b'-'
            || argument
                .iter()
                .position(|byte| *byte == b'=')
                .is_some_and(|position| position > 0)
    })
}

pub(super) fn apply_env(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for raw in stdout.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
            output.extend_from_slice(line);
            output.push(b'\n');
            continue;
        };
        let key = &line[..separator];
        let value = &line[separator + 1..];
        output.extend_from_slice(key);
        output.push(b'=');
        if env_sensitive_key(key) {
            if value.len() <= 4 {
                output.extend_from_slice(b"****");
            } else {
                output.extend_from_slice(&value[..2]);
                output.extend_from_slice(b"****");
                output.extend_from_slice(&value[value.len() - 2..]);
            }
        } else if value.len() > 100 {
            output.extend_from_slice(&value[..50]);
            output.extend_from_slice(b"...");
        } else {
            output.extend_from_slice(value);
        }
        output.push(b'\n');
    }
    output.extend_from_slice(stderr);
    output
}

fn env_sensitive_key(key: &[u8]) -> bool {
    [
        b"key".as_slice(),
        b"secret",
        b"password",
        b"token",
        b"credential",
        b"auth",
        b"private",
        b"api_key",
        b"apikey",
        b"access_key",
        b"jwt",
    ]
    .iter()
    .any(|needle| contains_ascii_case_insensitive(key, needle))
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}
