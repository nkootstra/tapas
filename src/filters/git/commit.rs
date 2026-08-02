pub(super) fn matches_commit(input: &[u8]) -> bool {
    first_nonempty_line(input).is_some_and(is_commit_summary_header)
}

fn is_commit_summary_header(line: &[u8]) -> bool {
    let Some(inner) = line
        .strip_prefix(b"[")
        .and_then(|line| line.split(|byte| *byte == b']').next())
    else {
        return false;
    };
    let Some(space) = inner.iter().rposition(|byte| *byte == b' ') else {
        return false;
    };
    let sha = &inner[space + 1..];
    sha.len() == 7 && sha.iter().all(u8::is_ascii_hexdigit)
}

pub(super) fn apply_commit(input: &[u8]) -> Vec<u8> {
    let mut lines = input.split(|byte| *byte == b'\n');
    let Some(header) = lines.find(|line| !line.is_empty()) else {
        return Vec::new();
    };
    let Some(bracket) = header.iter().position(|byte| *byte == b']') else {
        let mut output = header.to_vec();
        output.push(b'\n');
        return output;
    };
    let inner = &header[1..bracket];
    let Some(space) = inner.iter().rposition(|byte| *byte == b' ') else {
        return Vec::new();
    };
    let sha = &inner[space + 1..];
    let mut branch = &inner[..space];
    if let Some(paren) = find_subslice(branch, b" (") {
        branch = &branch[..paren];
    }
    let subject = header[bracket + 1..].trim_ascii_start();
    let mut output = Vec::with_capacity(input.len());
    output.extend_from_slice(b"c ");
    output.extend_from_slice(sha);
    output.push(b' ');
    output.extend_from_slice(branch);
    output.push(b' ');
    output.extend_from_slice(subject);
    output.push(b'\n');

    let mut stats_found = false;
    for line in lines.by_ref() {
        if line.is_empty() {
            continue;
        }
        let trimmed = line.trim_ascii_start();
        if find_subslice(trimmed, b" changed").is_some() {
            write_summary(&mut output, trimmed);
            stats_found = true;
            break;
        }
        output.extend_from_slice(line);
        output.push(b'\n');
    }
    if !stats_found {
        return group_file_entries(&output);
    }
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let trimmed = line.trim_ascii_start();
        if let Some(rest) = trimmed.strip_prefix(b"create mode ") {
            output.extend_from_slice(b"+ ");
            output.extend_from_slice(skip_mode_number(rest));
        } else if let Some(rest) = trimmed.strip_prefix(b"delete mode ") {
            output.extend_from_slice(b"- ");
            output.extend_from_slice(skip_mode_number(rest));
        } else {
            output.extend_from_slice(line);
        }
        output.push(b'\n');
    }
    group_file_entries(&output)
}

fn group_file_entries(input: &[u8]) -> Vec<u8> {
    let lines: Vec<&[u8]> = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .take(4096)
        .collect();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.len() >= 3 && matches!(line[0], b'+' | b'-') && line[1] == b' ' {
            let dir = parent_dir(&line[2..]);
            if !dir.is_empty() {
                let mut end = index + 1;
                while end < lines.len()
                    && lines[end].len() >= 3
                    && lines[end][0] == line[0]
                    && lines[end][1] == b' '
                    && parent_dir(&lines[end][2..]) == dir
                {
                    end += 1;
                }
                if end - index >= 3 {
                    output.extend_from_slice(&line[..2]);
                    output.extend_from_slice(dir);
                    output.extend_from_slice(b" x");
                    output.extend_from_slice((end - index).to_string().as_bytes());
                    output.push(b'\n');
                    index = end;
                    continue;
                }
            }
        }
        output.extend_from_slice(line);
        output.push(b'\n');
        index += 1;
    }
    output
}

pub(super) fn skip_mode_number(input: &[u8]) -> &[u8] {
    input
        .iter()
        .position(|byte| *byte == b' ')
        .map_or(input, |space| &input[space + 1..])
}

pub(super) fn write_summary(output: &mut Vec<u8>, line: &[u8]) {
    output.push(b'+');
    output.extend_from_slice(number_before_marker(line, b" insertion").unwrap_or(b"0"));
    output.extend_from_slice(b"/-");
    output.extend_from_slice(number_before_marker(line, b" deletion").unwrap_or(b"0"));
    output.extend_from_slice(b" files=");
    output.extend_from_slice(first_number(line).unwrap_or(b"0"));
    output.push(b'\n');
}

pub(super) fn first_number(input: &[u8]) -> Option<&[u8]> {
    let start = input.iter().position(u8::is_ascii_digit)?;
    let end = input[start..]
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .map_or(input.len(), |end| start + end);
    Some(&input[start..end])
}

fn number_before_marker<'a>(input: &'a [u8], marker: &[u8]) -> Option<&'a [u8]> {
    let mut end = find_subslice(input, marker)?;
    while end > 0 && input[end - 1] == b' ' {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && input[start - 1].is_ascii_digit() {
        start -= 1;
    }
    (start < end).then_some(&input[start..end])
}
use super::diff::first_nonempty_line;
use super::find_subslice;
use super::status::parent_dir;
