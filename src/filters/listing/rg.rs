fn rg_pattern_separator(line: &[u8]) -> Option<usize> {
    if line.is_empty() || matches!(line[0], b'{' | b' ' | b'\t') {
        return None;
    }
    let mut index = 1;
    while index < line.len() {
        if line[index] != b':' {
            index += 1;
            continue;
        }
        let mut digit = index + 1;
        if !line.get(digit).is_some_and(u8::is_ascii_digit) {
            index += 1;
            continue;
        }
        while line.get(digit).is_some_and(u8::is_ascii_digit) {
            digit += 1;
        }
        if line.get(digit) == Some(&b':') {
            return Some(index);
        }
        index += 1;
    }
    None
}

pub(super) fn rg_is_file_mode(argv: &[&[u8]]) -> bool {
    argv.iter()
        .any(|argument| matches!(*argument, b"--files" | b"-l" | b"--files-with-matches"))
}

pub(super) fn matches_rg_files(input: &[u8]) -> bool {
    if input.is_empty() {
        return false;
    }
    let first = input
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    if first.is_empty() || matches!(first[0], b' ' | b'\t' | 0..=0x1f) {
        return false;
    }
    !first.iter().enumerate().any(|(index, byte)| {
        *byte == b':' && index > 0 && first.get(index + 1).is_some_and(u8::is_ascii_digit)
    })
}

pub(super) fn apply_rg_files(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous_line: &[u8] = b"";
    let mut previous_dir_len = 0;
    let mut index = 0;
    while index < input.len() {
        let end = input[index..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(input.len(), |relative| index + relative);
        let line = &input[index..end];
        if line.starts_with(b":") {
            output.push(b':');
            output.extend_from_slice(line);
        } else if previous_dir_len > 0
            && line.len() > previous_dir_len
            && line.starts_with(&previous_line[..previous_dir_len])
            && line[previous_dir_len] != b':'
        {
            output.push(b':');
            output.extend_from_slice(&line[previous_dir_len..]);
        } else {
            output.extend_from_slice(line);
        }
        previous_line = line;
        previous_dir_len = line
            .iter()
            .rposition(|byte| *byte == b'/')
            .map_or(0, |separator| separator + 1);
        if end < input.len() {
            output.push(b'\n');
            index = end + 1;
        } else {
            break;
        }
    }
    output
}

pub(super) fn matches_rg_pattern(input: &[u8]) -> bool {
    input
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
        .and_then(rg_pattern_separator)
        .is_some()
}

pub(super) fn apply_rg_pattern(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous_path: Option<&[u8]> = None;
    let mut index = 0;
    while index < input.len() {
        let end = input[index..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(input.len(), |relative| index + relative);
        let line = &input[index..end];
        if let Some(separator) = rg_pattern_separator(line) {
            let path = &line[..separator];
            if previous_path == Some(path) {
                output.extend_from_slice(&line[separator..]);
            } else {
                output.extend_from_slice(line);
                previous_path = Some(path);
            }
        } else {
            output.extend_from_slice(line);
            previous_path = None;
        }
        if end < input.len() {
            output.push(b'\n');
            index = end + 1;
        } else {
            break;
        }
    }
    output
}
