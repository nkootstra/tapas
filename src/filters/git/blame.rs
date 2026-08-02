use super::*;

pub(super) fn matches_blame(input: &[u8]) -> bool {
    if input.len() < 60 {
        return false;
    }
    let mut checked = 0;
    let mut matched = 0;
    for line in input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .take(5)
    {
        checked += 1;
        let mut position = usize::from(line.starts_with(b"^"));
        while position < line.len() && line[position].is_ascii_hexdigit() {
            position += 1;
        }
        let sha_start = usize::from(line.starts_with(b"^"));
        if position - sha_start >= 7 && line[position..].trim_ascii_start().starts_with(b"(") {
            matched += 1;
        }
    }
    checked >= 2 && matched * 2 >= checked
}

pub(super) fn apply_blame(input: &[u8]) -> Vec<u8> {
    let mut expanded = Vec::with_capacity(input.len());
    let mut current_sha = None;
    let mut last_date: &[u8] = b"";
    let mut last_author: &[u8] = b"";

    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Some((sha, after_sha)) = parse_blame_sha(line) else {
            expanded.push(b' ');
            expanded.extend_from_slice(line);
            expanded.push(b'\n');
            continue;
        };
        let Some(paren_start) = after_sha.iter().position(|byte| *byte == b'(') else {
            expanded.push(b' ');
            expanded.extend_from_slice(after_sha);
            expanded.push(b'\n');
            continue;
        };
        let Some(relative_end) = after_sha[paren_start..]
            .iter()
            .position(|byte| *byte == b')')
        else {
            expanded.push(b' ');
            expanded.extend_from_slice(after_sha);
            expanded.push(b'\n');
            continue;
        };
        let paren_end = paren_start + relative_end;
        let metadata = &after_sha[paren_start + 1..paren_end];
        let after_paren = paren_end + 1;
        let code = if after_sha.get(after_paren) == Some(&b' ') {
            &after_sha[after_paren + 1..]
        } else {
            &after_sha[after_paren.min(after_sha.len())..]
        };
        let (author, date) = parse_blame_metadata(metadata);
        let Some(prefix) = sha.get(..7) else {
            expanded.push(b' ');
            expanded.extend_from_slice(after_sha);
            expanded.push(b'\n');
            continue;
        };
        let mut sha7 = [0_u8; 7];
        sha7.copy_from_slice(prefix);
        let first = current_sha.is_none();
        if current_sha != Some(sha7) {
            current_sha = Some(sha7);
            let author_out = if author.len() <= 20 {
                author
            } else {
                author.split(|byte| *byte == b' ').next().unwrap_or(author)
            };
            let emit_author = first || author_out != last_author;
            let emit_date = emit_author || date != last_date;
            expanded.extend_from_slice(b"b ");
            expanded.extend_from_slice(&sha7);
            if emit_date {
                expanded.push(b' ');
                expanded.extend_from_slice(date);
                last_date = date;
            }
            if emit_author {
                expanded.push(b' ');
                expanded.extend_from_slice(author_out);
                last_author = author_out;
            }
            expanded.push(b'\n');
        }
        expanded.push(b' ');
        expanded.extend_from_slice(code);
        expanded.push(b'\n');
    }
    truncate_blame_blocks(&expanded)
}

fn parse_blame_sha(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let start = usize::from(line.starts_with(b"^"));
    let mut end = start;
    while end < line.len() && line[end].is_ascii_hexdigit() {
        end += 1;
    }
    if end - start < 7 || line.get(end) != Some(&b' ') {
        return None;
    }
    Some((&line[start..end], &line[end..]))
}

fn parse_blame_metadata(content: &[u8]) -> (&[u8], &[u8]) {
    let trimmed = content.trim_ascii();
    let tokens: Vec<&[u8]> = trimmed
        .split(|byte| matches!(byte, b' ' | b'\t'))
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.len() < 4 {
        return (b"unknown", b"0000-00-00");
    }
    let date = tokens[tokens.len() - 4];
    let date_position = rfind_subslice(trimmed, date).unwrap_or(0);
    let author = trimmed[..date_position].trim_ascii_end();
    (
        if author.is_empty() {
            b"unknown"
        } else {
            author
        },
        date,
    )
}

fn truncate_blame_blocks(input: &[u8]) -> Vec<u8> {
    let lines: Vec<&[u8]> = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.starts_with(b"b ") {
            output.extend_from_slice(line);
            output.push(b'\n');
            index += 1;
            let source_start = index;
            while index < lines.len() && lines[index].starts_with(b" ") {
                index += 1;
            }
            let count = index - source_start;
            for source in &lines[source_start..source_start + count.min(3)] {
                output.extend_from_slice(source);
                output.push(b'\n');
            }
            if count > 3 {
                output.extend_from_slice(b" (+");
                output.extend_from_slice((count - 3).to_string().as_bytes());
                output.extend_from_slice(b")\n");
            }
        } else {
            output.extend_from_slice(line);
            output.push(b'\n');
            index += 1;
        }
    }
    output
}
