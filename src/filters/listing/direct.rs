use super::{
    EvidenceClass, StreamFilterDecision, StreamFilterInput, StreamFilterOutput, command_basename,
    find_subslice,
};

pub(super) fn dispatch(input: StreamFilterInput<'_>) -> StreamFilterDecision {
    if input.lossless || crate::invocation_policy::requests_passthrough(input.argv) {
        return StreamFilterDecision::Passthrough;
    }
    match command_basename(input.argv[0]) {
        b"diff" => dispatch_diff(input),
        b"head" | b"tail" => dispatch_file_slice(input),
        _ => StreamFilterDecision::Passthrough,
    }
}

fn dispatch_diff(input: StreamFilterInput<'_>) -> StreamFilterDecision {
    if input.exit_code > 1
        || input.argv.len() < 3
        || input.argv[1..]
            .iter()
            .any(|argument| argument.starts_with(b"-"))
        || std::str::from_utf8(input.stdout).is_err()
    {
        return StreamFilterDecision::Passthrough;
    }
    let compact = compact_normal_diff(input.stdout).or_else(|| compact_unified_diff(input.stdout));
    let Some(stdout) = compact else {
        return StreamFilterDecision::Passthrough;
    };
    StreamFilterDecision::Applied(StreamFilterOutput::new(
        stdout,
        input.stderr.to_vec(),
        EvidenceClass::FactComplete,
    ))
}

fn compact_normal_diff(input: &[u8]) -> Option<Vec<u8>> {
    let lines = logical_lines(input);
    let mut index = 0;
    let mut output = Vec::with_capacity(input.len());
    let mut hunks = 0;
    while index < lines.len() {
        let header = lines[index];
        let action = normal_header_action(header)?;
        output.push(b'@');
        output.extend_from_slice(header);
        output.push(b'\n');
        index += 1;
        let old_start = index;
        while lines.get(index).is_some_and(|line| line.starts_with(b"< ")) {
            output.push(b'-');
            output.extend_from_slice(&lines[index][2..]);
            output.push(b'\n');
            index += 1;
        }
        copy_no_newline_marker(&lines, &mut index, &mut output);
        if matches!(action, b'c') {
            if index == old_start || lines.get(index) != Some(&b"---".as_slice()) {
                return None;
            }
            index += 1;
        } else if action == b'd' && index == old_start || action == b'a' && index != old_start {
            return None;
        }
        let new_start = index;
        while lines.get(index).is_some_and(|line| line.starts_with(b"> ")) {
            output.push(b'+');
            output.extend_from_slice(&lines[index][2..]);
            output.push(b'\n');
            index += 1;
        }
        copy_no_newline_marker(&lines, &mut index, &mut output);
        if matches!(action, b'a' | b'c') && index == new_start
            || action == b'd' && index != new_start
        {
            return None;
        }
        hunks += 1;
    }
    (hunks > 0).then_some(output)
}

fn copy_no_newline_marker(lines: &[&[u8]], index: &mut usize, output: &mut Vec<u8>) {
    if lines
        .get(*index)
        .is_some_and(|line| line.starts_with(b"\\ No newline at end of file"))
    {
        output.extend_from_slice(lines[*index]);
        output.push(b'\n');
        *index += 1;
    }
}

fn normal_header_action(header: &[u8]) -> Option<u8> {
    let action_at = header
        .iter()
        .position(|byte| matches!(*byte, b'a' | b'c' | b'd'))?;
    let (old, rest) = header.split_at(action_at);
    let new = &rest[1..];
    (valid_line_range(old) && valid_line_range(new)).then_some(rest[0])
}

fn valid_line_range(value: &[u8]) -> bool {
    let mut parts = value.split(|byte| *byte == b',');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();
    !first.is_empty()
        && first.iter().all(u8::is_ascii_digit)
        && second.is_none_or(|part| !part.is_empty() && part.iter().all(u8::is_ascii_digit))
        && parts.next().is_none()
}

fn compact_unified_diff(input: &[u8]) -> Option<Vec<u8>> {
    let lines = logical_lines(input);
    let mut index = 0;
    let mut output = Vec::with_capacity(input.len());
    let mut files = 0;
    while index < lines.len() {
        let old = lines.get(index)?.strip_prefix(b"--- ")?;
        let new = lines.get(index + 1)?.strip_prefix(b"+++ ")?;
        if old.is_empty() || new.is_empty() {
            return None;
        }
        output.extend_from_slice(lines[index]);
        output.push(b'\n');
        output.extend_from_slice(lines[index + 1]);
        output.push(b'\n');
        index += 2;
        let mut hunks = 0;
        while index < lines.len() && !lines[index].starts_with(b"--- ") {
            let header = lines[index];
            let after_open = header.strip_prefix(b"@@ -")?;
            let close = find_subslice(after_open, b" @@")?;
            let coords = &after_open[..close];
            let split = find_subslice(coords, b" +")?;
            if !valid_unified_range(&coords[..split]) || !valid_unified_range(&coords[split + 2..])
            {
                return None;
            }
            output.push(b'@');
            output.extend_from_slice(&coords[..split]);
            output.push(b'|');
            output.extend_from_slice(&coords[split + 2..]);
            output.extend_from_slice(&after_open[close + 3..]);
            output.push(b'\n');
            index += 1;
            let content_start = index;
            while index < lines.len()
                && !lines[index].starts_with(b"@@ -")
                && !lines[index].starts_with(b"--- ")
            {
                if !matches!(lines[index].first(), Some(b' ' | b'+' | b'-' | b'\\')) {
                    return None;
                }
                output.extend_from_slice(lines[index]);
                output.push(b'\n');
                index += 1;
            }
            if index == content_start {
                return None;
            }
            hunks += 1;
        }
        if hunks == 0 {
            return None;
        }
        files += 1;
    }
    (files > 0).then_some(output)
}

fn valid_unified_range(value: &[u8]) -> bool {
    let value = value.strip_prefix(b"-").unwrap_or(value);
    valid_line_range(value)
}

fn dispatch_file_slice(input: StreamFilterInput<'_>) -> StreamFilterDecision {
    if input.exit_code != 0
        || input.argv.len() != 2
        || input.argv[1].starts_with(b"-")
        || !looks_like_text_file(input.argv[1])
    {
        return StreamFilterDecision::Passthrough;
    }
    let Some(stdout) = compact_text_slice(input.stdout) else {
        return StreamFilterDecision::Passthrough;
    };
    StreamFilterDecision::Applied(StreamFilterOutput::new(
        stdout,
        input.stderr.to_vec(),
        EvidenceClass::PotentiallyLossy,
    ))
}

fn compact_text_slice(input: &[u8]) -> Option<Vec<u8>> {
    std::str::from_utf8(input).ok()?;
    if input
        .iter()
        .any(|byte| *byte == 0 || *byte < b' ' && !matches!(*byte, b'\n' | b'\r' | b'\t'))
    {
        return None;
    }
    let lines = logical_lines(input);
    if lines.len() <= 8 {
        return None;
    }
    let omitted = lines.len() - 7;
    let mut output = Vec::with_capacity(input.len());
    for line in &lines[..4] {
        output.extend_from_slice(line);
        output.push(b'\n');
    }
    output.extend_from_slice(b"... ");
    output.extend_from_slice(omitted.to_string().as_bytes());
    output.extend_from_slice(b" lines omitted ...\n");
    for line in &lines[lines.len() - 3..] {
        output.extend_from_slice(line);
        output.push(b'\n');
    }
    Some(output)
}

fn looks_like_text_file(path: &[u8]) -> bool {
    let Some(extension) = path.rsplit(|byte| *byte == b'.').next() else {
        return false;
    };
    [
        b"txt".as_slice(),
        b"md",
        b"rs",
        b"zig",
        b"go",
        b"py",
        b"ts",
        b"tsx",
        b"js",
        b"jsx",
        b"java",
        b"kt",
        b"c",
        b"h",
        b"cpp",
        b"rb",
        b"sh",
        b"zsh",
        b"fish",
        b"yaml",
        b"yml",
        b"toml",
        b"json",
        b"xml",
        b"html",
        b"css",
        b"sql",
        b"env",
        b"ini",
        b"cfg",
        b"conf",
    ]
    .contains(&extension)
}

fn logical_lines(input: &[u8]) -> Vec<&[u8]> {
    let input = input.strip_suffix(b"\n").unwrap_or(input);
    if input.is_empty() {
        Vec::new()
    } else {
        input.split(|byte| *byte == b'\n').collect()
    }
}
