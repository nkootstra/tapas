pub(super) fn matches_merge(input: &[u8]) -> bool {
    first_nonempty_line(input).is_some_and(|line| {
        line.starts_with(b"Merge made by")
            || (line.starts_with(b"Updating ") && find_subslice(line, b"..").is_some())
            || line.starts_with(b"Already up to date")
    })
}

pub(super) fn apply_merge(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let source = if stdout.is_empty() { stderr } else { stdout };
    if source.is_empty() && stderr.is_empty() {
        return Vec::new();
    }
    let lines: Vec<&[u8]> = source.split(|byte| *byte == b'\n').collect();
    let Some(first_index) = lines.iter().position(|line| !line.is_empty()) else {
        return Vec::new();
    };
    let first = lines[first_index];
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    if let Some(range) = first.strip_prefix(b"Updating ") {
        if let Some(dots) = find_subslice(range, b"..") {
            output.extend_from_slice(b"@ ff ");
            output.extend_from_slice(&range[..dots.min(7)]);
            output.extend_from_slice(b"..");
            let right = &range[dots + 2..];
            let end = right
                .iter()
                .position(|byte| *byte == b' ')
                .unwrap_or(right.len());
            output.extend_from_slice(&right[..end.min(7)]);
            output.push(b'\n');
        }
        let body_start = lines[first_index + 1..]
            .iter()
            .position(|line| *line == b"Fast-forward")
            .map_or(lines.len(), |index| first_index + 2 + index);
        emit_merge_body(&mut output, &lines[body_start..]);
    } else if first.starts_with(b"Merge made by") {
        let strategy = first
            .iter()
            .position(|byte| *byte == b'\'')
            .and_then(|start| {
                let rest = &first[start + 1..];
                rest.iter()
                    .position(|byte| *byte == b'\'')
                    .map(|end| &rest[..end])
            })
            .unwrap_or(b"ort");
        output.extend_from_slice(b"@ merge ");
        output.extend_from_slice(strategy);
        output.push(b'\n');
        emit_merge_body(&mut output, &lines[first_index + 1..]);
    } else if first.starts_with(b"Already up to date") {
        output.extend_from_slice(b"up to date\n");
    } else {
        emit_merge_conflicts(&mut output, &lines[first_index + 1..]);
    }
    if !stdout.is_empty() && !stderr.is_empty() {
        let stderr_lines: Vec<&[u8]> = stderr.split(|byte| *byte == b'\n').collect();
        emit_merge_conflicts(&mut output, &stderr_lines);
    }
    group_merge_entries(&output)
}

fn emit_merge_body(output: &mut Vec<u8>, lines: &[&[u8]]) {
    for line in lines {
        let line = line.trim_ascii_start();
        if line.is_empty() {
            continue;
        }
        if line.starts_with(b"CONFLICT (") {
            emit_conflict(output, line);
        } else if line.starts_with(b"Automatic merge failed") {
            output.extend_from_slice(b"! failed\n");
        } else if let Some(rest) = line.strip_prefix(b"create mode ") {
            output.extend_from_slice(b"+ ");
            output.extend_from_slice(skip_mode_number(rest));
            output.push(b'\n');
        } else if let Some(rest) = line.strip_prefix(b"delete mode ") {
            output.extend_from_slice(b"- ");
            output.extend_from_slice(skip_mode_number(rest));
            output.push(b'\n');
        } else if find_subslice(line, b" | ").is_some() {
            write_stat_line(output, line);
        } else if find_subslice(line, b" changed").is_some() {
            write_summary(output, line);
        }
    }
}

fn emit_merge_conflicts(output: &mut Vec<u8>, lines: &[&[u8]]) {
    for line in lines {
        let line = line.trim_ascii_start();
        if line.starts_with(b"CONFLICT (") {
            emit_conflict(output, line);
        } else if line.starts_with(b"Automatic merge failed") {
            output.extend_from_slice(b"! failed\n");
        }
    }
}

pub(super) fn emit_conflict(output: &mut Vec<u8>, line: &[u8]) {
    let path = rfind_subslice(line, b" in ")
        .map(|position| line[position + 4..].trim_ascii())
        .or_else(|| find_subslice(line, b": ").map(|position| line[position + 2..].trim_ascii()))
        .unwrap_or_default();
    if !path.is_empty() {
        output.extend_from_slice(b"! conflict ");
        output.extend_from_slice(path);
        output.push(b'\n');
    }
}

fn write_stat_line(output: &mut Vec<u8>, line: &[u8]) {
    let Some(pipe) = find_subslice(line, b" | ") else {
        output.extend_from_slice(line);
        output.push(b'\n');
        return;
    };
    let after = &line[pipe + 3..];
    let digits = after
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(after.len());
    output.extend_from_slice(&line[..pipe]);
    output.extend_from_slice(b" |");
    output.extend_from_slice(&after[..digits]);
    output.push(b'\n');
}

fn group_merge_entries(input: &[u8]) -> Vec<u8> {
    let lines: Vec<&[u8]> = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .take(4096)
        .collect();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let stat_dir = merge_stat_dir(line);
        let create_dir = merge_create_dir(line);
        if let Some(dir) = stat_dir.or(create_dir)
            && !dir.is_empty()
        {
            let grouping_stats = stat_dir.is_some();
            let mut end = index + 1;
            while end < lines.len() {
                let next = if grouping_stats {
                    merge_stat_dir(lines[end])
                } else {
                    merge_create_dir(lines[end])
                };
                if next != Some(dir) {
                    break;
                }
                end += 1;
            }
            if end - index >= 3 {
                if !grouping_stats {
                    output.extend_from_slice(&line[..2]);
                }
                output.extend_from_slice(dir);
                output.extend_from_slice(b" x");
                output.extend_from_slice((end - index).to_string().as_bytes());
                output.push(b'\n');
                index = end;
                continue;
            }
        }
        output.extend_from_slice(line);
        output.push(b'\n');
        index += 1;
    }
    output
}

fn merge_stat_dir(line: &[u8]) -> Option<&[u8]> {
    let pipe = find_subslice(line, b" |")?;
    let path = &line[..pipe];
    let slash = path.iter().rposition(|byte| *byte == b'/')?;
    Some(&path[..=slash])
}

fn merge_create_dir(line: &[u8]) -> Option<&[u8]> {
    if line.len() < 3 || !matches!(line[0], b'+' | b'-') || line[1] != b' ' {
        return None;
    }
    let path = &line[2..];
    let slash = path.iter().rposition(|byte| *byte == b'/')?;
    Some(&path[..=slash])
}
use super::commit::{skip_mode_number, write_summary};
use super::diff::first_nonempty_line;
use super::{find_subslice, rfind_subslice};
