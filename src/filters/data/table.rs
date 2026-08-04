pub(super) fn matches_pup_table(input: &[u8]) -> bool {
    let mut saw_border = false;
    let mut saw_row = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if is_pup_border(line) {
            saw_border = true;
        } else if is_pup_pipe_row(line) {
            saw_row = true;
        }
    }
    saw_border && saw_row
}

pub(super) fn compact_pup_table(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.strip_suffix(b"\r").unwrap_or(&clean);
        if line.is_empty() || is_pup_border(line) || is_pup_separator(line) {
            continue;
        }
        if !is_pup_pipe_row(line) {
            output.extend_from_slice(line);
            output.push(b'\n');
            continue;
        }

        let start = usize::from(line.first() == Some(&b'|'));
        let end = if line.len() > start && line.last() == Some(&b'|') {
            line.len() - 1
        } else {
            line.len()
        };
        let mut fields = line[start..end]
            .split(|byte| *byte == b'|')
            .map(|field| field.trim_ascii())
            .collect::<Vec<_>>();
        while fields.last().is_some_and(|field| field.is_empty()) {
            fields.pop();
        }
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                output.push(b'\t');
            }
            output.extend_from_slice(field);
        }
        if !fields.is_empty() {
            output.push(b'\n');
        }
    }
    output
}

fn is_pup_pipe_row(line: &[u8]) -> bool {
    line.len() >= 2 && line[0] == b'|' && line[1..].contains(&b'|')
}

fn is_pup_border(line: &[u8]) -> bool {
    !line.is_empty()
        && line[0] == b'+'
        && line.iter().all(|byte| matches!(byte, b'+' | b'-' | b'='))
}

fn is_pup_separator(line: &[u8]) -> bool {
    is_pup_pipe_row(line) && line.iter().all(|byte| matches!(byte, b'|' | b'-'))
}

pub(super) fn matches_columnar(input: &[u8]) -> bool {
    input.windows(2).any(|window| window == b"  ")
}

pub(super) fn matches_sqlite_table(input: &[u8]) -> bool {
    sqlite_columns(input).is_some_and(|(separator, header, rows)| {
        header.len() > 1
            && rows.len() > 1
            && input
                .split(|byte| *byte == b'\n')
                .enumerate()
                .skip(separator + 1)
                .any(|(_, line)| !line.trim_ascii().is_empty())
    })
}

pub(super) fn compact_sqlite_table(input: &[u8]) -> Vec<u8> {
    let Some((separator, header_starts, row_starts)) = sqlite_columns(input) else {
        return input.to_vec();
    };
    let mut output = Vec::with_capacity(input.len());
    for (line_index, raw) in input.split(|byte| *byte == b'\n').enumerate() {
        if raw.trim_ascii().is_empty() || line_index == separator {
            continue;
        }
        let starts = if line_index < separator {
            &header_starts
        } else {
            &row_starts
        };
        for (index, start) in starts.iter().enumerate() {
            if index > 0 {
                output.push(b'\t');
            }
            let start = (*start).min(raw.len());
            let end = starts
                .get(index + 1)
                .copied()
                .unwrap_or(raw.len())
                .min(raw.len())
                .max(start);
            output.extend_from_slice(raw[start..end].trim_ascii());
        }
        output.push(b'\n');
    }
    output
}

fn sqlite_columns(input: &[u8]) -> Option<(usize, Vec<usize>, Vec<usize>)> {
    let mut nonempty_lines = 0;
    let mut header = None;
    for (index, raw) in input.split(|byte| *byte == b'\n').enumerate() {
        let line = raw.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if nonempty_lines == 0 {
            header = Some(raw);
        } else if nonempty_lines == 1 && is_sqlite_separator(line) {
            return Some((
                index,
                sqlite_header_starts(header?),
                sqlite_column_starts(raw),
            ));
        }
        nonempty_lines += 1;
    }
    None
}

fn sqlite_header_starts(line: &[u8]) -> Vec<usize> {
    let first = line
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(line.len());
    if first == line.len() {
        return Vec::new();
    }
    let mut starts = vec![first];
    let mut index = first;
    while index + 1 < line.len() {
        if line[index].is_ascii_whitespace() && line[index + 1].is_ascii_whitespace() {
            let mut next = index + 2;
            while next < line.len() && line[next].is_ascii_whitespace() {
                next += 1;
            }
            if next < line.len() {
                starts.push(next);
                index = next;
                continue;
            }
        }
        index += 1;
    }
    starts
}

fn sqlite_column_starts(line: &[u8]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut start = None;
    for (index, byte) in line.iter().enumerate() {
        if *byte == b'-' {
            start.get_or_insert(index);
        } else if let Some(begin) = start.take() {
            starts.push(begin);
        }
    }
    if let Some(begin) = start {
        starts.push(begin);
    }
    starts
}

fn is_sqlite_separator(line: &[u8]) -> bool {
    line.contains(&b'-') && line.iter().all(|byte| matches!(byte, b'-' | b' ' | b'\t'))
}

pub(super) fn compact_columnar(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous: Vec<&[u8]> = Vec::new();
    let mut repeated_rows = 0usize;
    let mut offset = 0usize;

    while offset < input.len() {
        let relative_end = input[offset..].iter().position(|byte| *byte == b'\n');
        let end = relative_end.map_or(input.len(), |index| offset + index);
        let line = &input[offset..end];
        let has_newline = end < input.len();

        if !line.windows(2).any(|window| window == b"  ") {
            flush_repeated_rows(&mut output, &mut repeated_rows);
            output.extend_from_slice(line);
            if has_newline {
                output.push(b'\n');
            }
            previous.clear();
        } else {
            let fields = split_columnar_fields(line);
            if !previous.is_empty() && fields == previous {
                repeated_rows += 1;
            } else {
                flush_repeated_rows(&mut output, &mut repeated_rows);
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        output.push(b' ');
                    }
                    if !previous.is_empty()
                        && previous.get(index).is_some_and(|prior| prior == field)
                        && !field.is_empty()
                    {
                        output.push(b'~');
                    } else if index + 1 == fields.len() {
                        write_truncated_last_field(&mut output, field);
                    } else {
                        output.extend_from_slice(field);
                    }
                }
                if has_newline {
                    output.push(b'\n');
                }
                previous = fields;
            }
        }
        offset = end + usize::from(has_newline);
    }
    flush_repeated_rows(&mut output, &mut repeated_rows);
    output
}

fn split_columnar_fields(line: &[u8]) -> Vec<&[u8]> {
    let mut fields = Vec::new();
    let mut index = 0usize;
    while index < line.len() && line[index] == b' ' {
        index += 1;
    }
    while index < line.len() {
        let start = index;
        while index < line.len() {
            if line[index] != b' ' {
                index += 1;
                continue;
            }
            let mut after = index;
            while after < line.len() && line[after] == b' ' {
                after += 1;
            }
            if after - index >= 2 {
                break;
            }
            index = after;
        }
        if index > start {
            fields.push(&line[start..index]);
        }
        while index < line.len() && line[index] == b' ' {
            index += 1;
        }
    }
    fields
}

fn flush_repeated_rows(output: &mut Vec<u8>, repeated_rows: &mut usize) {
    if *repeated_rows > 0 {
        output.extend_from_slice(b"~ x");
        output.extend_from_slice(repeated_rows.to_string().as_bytes());
        output.push(b'\n');
        *repeated_rows = 0;
    }
}

fn write_truncated_last_field(output: &mut Vec<u8>, field: &[u8]) {
    if field.first() == Some(&b'/') {
        if let Some(slash) = field.iter().rposition(|byte| *byte == b'/')
            && slash + 1 < field.len()
        {
            output.extend_from_slice(&field[slash + 1..]);
            return;
        }
    } else if let Some(space) = field.windows(2).position(|window| window == b" /") {
        output.extend_from_slice(&field[..=space]);
        let path = &field[space + 1..];
        if let Some(slash) = path.iter().rposition(|byte| *byte == b'/')
            && slash + 1 < path.len()
        {
            output.extend_from_slice(&path[slash + 1..]);
            return;
        }
        output.extend_from_slice(path);
        return;
    }
    output.extend_from_slice(field);
}

pub(super) fn is_columnar_command(command: &[u8]) -> bool {
    matches!(
        command,
        b"docker"
            | b"docker-compose"
            | b"kubectl"
            | b"gh"
            | b"ps"
            | b"df"
            | b"psql"
            | b"systemctl"
            | b"lsof"
            | b"npm"
            | b"pnpm"
            | b"yarn"
            | b"brew"
            | b"bun"
    )
}
use super::strip_ansi;
