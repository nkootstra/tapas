pub(super) fn aws_requests_table(argv: &[&[u8]]) -> bool {
    let mut table_outputs = 0usize;
    let mut index = 1usize;
    while index < argv.len() {
        let argument = argv[index];
        if argument == b"--" {
            break;
        }
        if argument == b"--output" {
            let Some(value) = argv.get(index + 1) else {
                return false;
            };
            if *value != b"table" {
                return false;
            }
            table_outputs += 1;
            index += 2;
            continue;
        }
        if let Some(value) = argument.strip_prefix(b"--output=") {
            if value != b"table" {
                return false;
            }
            table_outputs += 1;
        } else if matches!(
            argument,
            b"--query" | b"--cli-binary-format" | b"--generate-cli-skeleton"
        ) || argument.starts_with(b"--query=")
            || argument.starts_with(b"--cli-binary-format=")
            || argument.starts_with(b"--generate-cli-skeleton=")
        {
            return false;
        }
        index += 1;
    }
    table_outputs == 1
}

pub(super) fn matches_aws_table(input: &[u8]) -> bool {
    if std::str::from_utf8(input).is_err() {
        return false;
    }
    let mut borders = 0usize;
    let mut rows = 0usize;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw).trim_ascii();
        if line.is_empty() {
            continue;
        }
        if is_aws_border(line) {
            borders += 1;
        } else if aws_fields(line).is_some() {
            rows += 1;
        } else {
            return false;
        }
    }
    borders >= 2 && rows >= 2
}

pub(super) fn compact_aws_table(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw).trim_ascii();
        if line.is_empty() || is_aws_border(line) {
            continue;
        }
        let Some(fields) = aws_fields(line) else {
            return input.to_vec();
        };
        write_fields(&mut output, fields);
    }
    output
}

fn is_aws_border(line: &[u8]) -> bool {
    line.contains(&b'-')
        && line
            .iter()
            .all(|byte| matches!(byte, b'+' | b'-' | b'=' | b'|'))
}

fn aws_fields(line: &[u8]) -> Option<Vec<&[u8]>> {
    if line.first() != Some(&b'|') || line.last() != Some(&b'|') {
        return None;
    }
    let fields = line
        .split(|byte| *byte == b'|')
        .map(|field| field.trim_ascii())
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    (!fields.is_empty()).then_some(fields)
}

pub(super) fn is_psql_table_route(argv: &[&[u8]]) -> bool {
    match argv {
        [_, option] => {
            matches!(*option, b"-l" | b"--list")
                || option.starts_with(b"-c") && option.len() > 2
                || option.starts_with(b"--command=") && option.len() > b"--command=".len()
        }
        [_, option, command] => matches!(*option, b"-c" | b"--command") && !command.is_empty(),
        _ => false,
    }
}

pub(super) fn matches_psql_table(input: &[u8]) -> bool {
    psql_table_parts(input).is_some()
}

pub(super) fn compact_psql_table(input: &[u8]) -> Vec<u8> {
    let Some((separator, end)) = psql_table_parts(input) else {
        return input.to_vec();
    };
    let lines = input
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .collect::<Vec<_>>();
    let mut output = Vec::with_capacity(input.len());
    for line in lines[..separator]
        .iter()
        .filter(|line| !line.trim_ascii().is_empty())
    {
        let line = line.trim_ascii();
        if line.contains(&b'|') {
            write_fields(&mut output, pipe_fields(line));
        } else {
            output.extend_from_slice(line);
            output.push(b'\n');
        }
    }
    for line in &lines[separator + 1..end] {
        write_fields(&mut output, pipe_fields(line.trim_ascii()));
    }
    for line in &lines[end..] {
        let line = line.trim_ascii();
        if !line.is_empty() {
            output.extend_from_slice(line);
            output.push(b'\n');
        }
    }
    output
}

fn psql_table_parts(input: &[u8]) -> Option<(usize, usize)> {
    if std::str::from_utf8(input).is_err() {
        return None;
    }
    let lines = input
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .collect::<Vec<_>>();
    let separator = lines
        .iter()
        .position(|line| is_psql_separator(line.trim_ascii()))?;
    let header = lines[..separator]
        .iter()
        .rfind(|line| !line.trim_ascii().is_empty())?
        .trim_ascii();
    let columns = pipe_fields(header).len();
    if columns < 2 {
        return None;
    }
    let mut end = separator + 1;
    while end < lines.len() {
        let line = lines[end].trim_ascii();
        if line.is_empty() || is_psql_footer(line) {
            break;
        }
        if pipe_fields(line).len() != columns {
            return None;
        }
        end += 1;
    }
    if end == separator + 1
        || lines[end..]
            .iter()
            .map(|line| line.trim_ascii())
            .any(|line| !line.is_empty() && !is_psql_footer(line))
    {
        return None;
    }
    Some((separator, end))
}

fn is_psql_separator(line: &[u8]) -> bool {
    line.contains(&b'+')
        && line.contains(&b'-')
        && line.iter().all(|byte| matches!(byte, b'+' | b'-'))
}

fn is_psql_footer(line: &[u8]) -> bool {
    let Some(inner) = line
        .strip_prefix(b"(")
        .and_then(|line| line.strip_suffix(b")"))
    else {
        return false;
    };
    let Some(space) = inner.iter().position(|byte| *byte == b' ') else {
        return false;
    };
    let count = &inner[..space];
    let noun = &inner[space + 1..];
    !count.is_empty() && count.iter().all(u8::is_ascii_digit) && matches!(noun, b"row" | b"rows")
}

fn pipe_fields(line: &[u8]) -> Vec<&[u8]> {
    line.split(|byte| *byte == b'|')
        .map(|field| field.trim_ascii())
        .collect()
}

fn write_fields(output: &mut Vec<u8>, fields: Vec<&[u8]>) {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            output.push(b'\t');
        }
        output.extend_from_slice(field);
    }
    output.push(b'\n');
}

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
