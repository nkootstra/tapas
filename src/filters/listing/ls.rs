pub(super) fn ls_wants_columns(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        if let Some(format) = argument.strip_prefix(b"--format=") {
            return matches!(format, b"across" | b"commas" | b"horizontal" | b"vertical");
        }
        argument.len() >= 2
            && argument[0] == b'-'
            && argument[1] != b'-'
            && argument[1..]
                .iter()
                .any(|flag| matches!(flag, b'C' | b'x' | b'm'))
    })
}

pub(super) fn apply_ls_plain(input: &[u8], columns: bool) -> Option<Vec<u8>> {
    if input.is_empty() {
        return Some(Vec::new());
    }
    if ls_looks_like_blocks(input) {
        apply_ls_blocks(input, columns)
    } else {
        Some(apply_ls_flat(input, columns))
    }
}

fn ls_looks_like_blocks(input: &[u8]) -> bool {
    let mut previous_blank = true;
    let mut saw_header = false;
    let mut saw_content = false;
    let mut pending_blank = false;
    let mut saw_interior_blank = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            previous_blank = true;
            if saw_content {
                pending_blank = true;
            }
            continue;
        }
        if pending_blank {
            saw_interior_blank = true;
        }
        pending_blank = false;
        if previous_blank && line.len() >= 2 && line.ends_with(b":") {
            saw_header = true;
        }
        previous_blank = false;
        saw_content = true;
    }
    saw_header && saw_interior_blank
}

fn apply_ls_flat(input: &[u8], columns: bool) -> Vec<u8> {
    let mut names = Vec::new();
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        if columns {
            tokenize_ls_row(line, &mut names);
        } else {
            names.push(line);
        }
    }
    if columns {
        names.sort();
    }
    let mut output = Vec::with_capacity(input.len());
    for name in names {
        if matches!(name, b"." | b"..") {
            continue;
        }
        output.extend_from_slice(name);
        output.push(b'\n');
    }
    output
}

fn tokenize_ls_row<'a>(line: &'a [u8], output: &mut Vec<&'a [u8]>) {
    let mut index = 0;
    let mut start = None;
    while index < line.len() {
        match line[index] {
            b' ' => {
                let mut end = index;
                while line.get(end) == Some(&b' ') {
                    end += 1;
                }
                if end - index >= 2
                    && let Some(token_start) = start.take()
                {
                    push_ls_token(&line[token_start..index], output);
                }
                index = end;
            }
            b'\t' => {
                if let Some(token_start) = start.take() {
                    push_ls_token(&line[token_start..index], output);
                }
                index += 1;
            }
            b',' if index + 1 == line.len() || line.get(index + 1) == Some(&b' ') => {
                if let Some(token_start) = start.take() {
                    push_ls_token(&line[token_start..index], output);
                }
                index += 1;
            }
            _ => {
                start.get_or_insert(index);
                index += 1;
            }
        }
    }
    if let Some(token_start) = start {
        push_ls_token(&line[token_start..], output);
    }
}

fn push_ls_token<'a>(token: &'a [u8], output: &mut Vec<&'a [u8]>) {
    let token = token.trim_ascii();
    if !token.is_empty() {
        output.push(token);
    }
}

fn apply_ls_blocks(input: &[u8], columns: bool) -> Option<Vec<u8>> {
    type Segment<'a> = (Option<&'a [u8]>, Vec<&'a [u8]>);

    let mut header = None;
    let mut entries = Vec::new();
    let mut segments: Vec<Segment<'_>> = Vec::new();
    let mut previous_blank = true;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            previous_blank = true;
            continue;
        }
        if previous_blank && line.len() >= 2 && line.ends_with(b":") {
            if columns {
                entries.sort();
            }
            segments.push((header, std::mem::take(&mut entries)));
            header = Some(&line[..line.len() - 1]);
            previous_blank = false;
            continue;
        }
        previous_blank = false;
        if columns {
            tokenize_ls_row(line, &mut entries);
        } else {
            entries.push(line);
        }
    }
    if columns {
        entries.sort();
    }
    segments.push((header, entries));

    let mut output = Vec::with_capacity(input.len());
    for (header, entries) in segments {
        flush_ls_segment(&mut output, header, &entries);
    }
    if output.is_empty() {
        None
    } else {
        output.push(b'\n');
        Some(output)
    }
}

fn flush_ls_segment(output: &mut Vec<u8>, header: Option<&[u8]>, entries: &[&[u8]]) {
    let real: Vec<_> = entries
        .iter()
        .copied()
        .filter(|entry| !matches!(*entry, b"." | b".."))
        .collect();
    if real.is_empty() {
        return;
    }
    let Some(header) = header else {
        for entry in real {
            write_output_line(output, entry);
        }
        return;
    };
    if real.len() >= 3 {
        start_output_line(output);
        output.extend_from_slice(header);
        output.extend_from_slice(b"/ (");
        output.extend_from_slice(real.len().to_string().as_bytes());
        output.extend_from_slice(b" entries: ");
        for (position, entry) in real.iter().take(3).enumerate() {
            if position > 0 {
                output.extend_from_slice(b", ");
            }
            output.extend_from_slice(entry);
        }
        write_omission(output, real.len(), 3);
        output.push(b')');
    } else {
        for entry in real {
            start_output_line(output);
            output.extend_from_slice(header);
            output.push(b'/');
            output.extend_from_slice(entry);
        }
    }
}
use super::pipe::write_omission;
use super::tree::{start_output_line, write_output_line};
