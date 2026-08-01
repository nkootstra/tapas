pub(crate) fn command_basename(command: &[u8]) -> &[u8] {
    command
        .iter()
        .rposition(|byte| matches!(byte, b'/' | b'\\'))
        .map_or(command, |separator| &command[separator + 1..])
}

pub(crate) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

pub(crate) fn rfind_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(haystack.len());
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

pub(crate) fn append_line(output: &mut Vec<u8>, line: &[u8]) {
    output.extend_from_slice(line);
    output.push(b'\n');
}

pub(crate) fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

pub(crate) fn byte_after_lines(data: &[u8], line_count: usize) -> usize {
    if line_count == 0 {
        return 0;
    }
    data.iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'\n')
        .nth(line_count - 1)
        .map_or(data.len(), |(index, _)| index + 1)
}

pub(crate) fn trim_ascii_end_space(mut input: &[u8]) -> &[u8] {
    while input
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[..input.len() - 1];
    }
    input
}

pub(crate) fn normalize_log_line(line: &[u8], compose: bool) -> Vec<u8> {
    if compose && let Some(pipe) = line.iter().position(|byte| *byte == b'|') {
        let service = line[..pipe].trim_ascii();
        if !service.is_empty() {
            let payload = line[pipe + 1..].trim_ascii_start();
            let payload = &payload[timestamp_end(payload)..];
            let mut normalized = service.to_vec();
            normalized.push(b'|');
            if !payload.is_empty() {
                normalized.push(b' ');
                normalized.extend_from_slice(payload);
            }
            return normalized;
        }
    }
    line.to_vec()
}

pub(crate) fn timestamp_end(line: &[u8]) -> usize {
    if line.len() < 10
        || !line[..4].iter().all(u8::is_ascii_digit)
        || line[4] != b'-'
        || !line[5..7].iter().all(u8::is_ascii_digit)
        || line[7] != b'-'
        || !line[8..10].iter().all(u8::is_ascii_digit)
    {
        return 0;
    }
    let mut cursor = if line.get(10) == Some(&b' ')
        && line.len() >= 19
        && line[11..13].iter().all(u8::is_ascii_digit)
        && line[13] == b':'
        && line[14..16].iter().all(u8::is_ascii_digit)
        && line[16] == b':'
        && line[17..19].iter().all(u8::is_ascii_digit)
    {
        19
    } else {
        0
    };
    while cursor < line.len() && !matches!(line[cursor], b' ' | b'\t') {
        cursor += 1;
    }
    while line
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    cursor
}

pub(crate) fn strip_ansi(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0usize;
    while index < input.len() {
        let Some(relative) = input[index..].iter().position(|byte| *byte == 0x1b) else {
            output.extend_from_slice(&input[index..]);
            break;
        };
        let escape = index + relative;
        output.extend_from_slice(&input[index..escape]);
        index = escape;
        match input.get(index + 1) {
            Some(b'[') => {
                index += 2;
                while index < input.len() && !(0x40..=0x7e).contains(&input[index]) {
                    index += 1;
                }
                index += usize::from(index < input.len());
            }
            Some(b']') => {
                index += 2;
                while index < input.len() {
                    if input[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if input[index] == 0x1b && input.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            Some(_) => index += 2,
            None => index += 1,
        }
    }
    output
}

pub(crate) fn strip_ansi_csi(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        if input[cursor] == 0x1b && input.get(cursor + 1) == Some(&b'[') {
            cursor += 2;
            while cursor < input.len() {
                let byte = input[cursor];
                cursor += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        } else {
            output.push(input[cursor]);
            cursor += 1;
        }
    }
    output
}
