use super::*;

pub(super) fn matches_log(input: &[u8]) -> bool {
    first_nonempty_line(input).is_some_and(is_commit_line)
}

pub(super) fn matches_show(input: &[u8]) -> bool {
    matches_log(input) && find_diff_start(&input[..input.len().min(8 * 1024)]).is_some()
}

pub(super) fn apply_show(input: &[u8]) -> Vec<u8> {
    let Some(diff_start) = find_diff_start(input) else {
        return apply_log_with_body(input);
    };
    let mut header_end = diff_start;
    while header_end > 0 && matches!(input[header_end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        header_end -= 1;
    }
    let mut output = apply_log_with_body(&input[..header_end]);
    output.push(b'\n');
    output.extend_from_slice(&apply_diff(&input[diff_start..]));
    output
}

pub(super) fn find_diff_start(input: &[u8]) -> Option<usize> {
    let marker = b"diff --git a/";
    let mut offset = 0;
    while offset < input.len() {
        let found = find_subslice(&input[offset..], marker)?;
        let absolute = offset + found;
        if absolute == 0 || input[absolute - 1] == b'\n' {
            return Some(absolute);
        }
        offset = absolute + marker.len();
    }
    None
}

fn apply_log_with_body(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut sha7 = None;
    let mut subject_emitted = false;
    let mut in_body = false;
    let mut merge_parents = None;

    for line in input.split(|byte| *byte == b'\n') {
        if is_commit_line(line) {
            sha7 = line
                .get(b"commit ".len()..b"commit ".len() + 7)
                .and_then(|sha| <[u8; 7]>::try_from(sha).ok());
            subject_emitted = false;
            in_body = false;
            merge_parents = None;
            continue;
        }
        let Some(commit_sha) = sha7 else {
            continue;
        };
        if !subject_emitted {
            if let Some(parents) = line.strip_prefix(b"Merge: ") {
                merge_parents = Some(parents);
                continue;
            }
            let Some(body) = line.strip_prefix(b"    ") else {
                continue;
            };
            let subject = body.trim_ascii();
            if subject.is_empty() {
                continue;
            }
            if !output.is_empty() {
                output.push(b'\n');
            }
            output.extend_from_slice(&commit_sha);
            output.push(b' ');
            output.extend_from_slice(subject);
            if let Some(parents) = merge_parents {
                output.extend_from_slice(b"\np ");
                output.extend_from_slice(parents);
            }
            subject_emitted = true;
            in_body = true;
            continue;
        }
        if in_body {
            if let Some(body) = line.strip_prefix(b"    ") {
                let body = body.trim_ascii();
                if !body.is_empty() {
                    output.extend_from_slice(b"\n  ");
                    output.extend_from_slice(body);
                }
            } else if !line.is_empty() {
                in_body = false;
            }
        }
    }
    if !output.is_empty() {
        output.push(b'\n');
    }
    output
}

fn is_commit_line(line: &[u8]) -> bool {
    let Some(rest) = line.strip_prefix(b"commit ") else {
        return false;
    };
    rest.len() >= 40
        && rest[..40].iter().all(u8::is_ascii_hexdigit)
        && rest.get(40).is_none_or(|byte| matches!(byte, b' ' | b'\t'))
}

#[derive(Default)]
struct CompactCommit {
    sha7: Option<[u8; 7]>,
    subject: Vec<u8>,
    important: Vec<u8>,
}

pub(super) fn apply_log_compact(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut current = CompactCommit::default();

    for line in input.split(|byte| *byte == b'\n') {
        if is_commit_line(line) {
            flush_compact_commit(&mut output, &current);
            current = CompactCommit::default();
            current.sha7 = line
                .get(b"commit ".len()..b"commit ".len() + 7)
                .and_then(|sha| sha.try_into().ok());
            continue;
        }
        if current.sha7.is_none() {
            continue;
        }
        let Some(body) = line.strip_prefix(b"    ") else {
            continue;
        };
        let body = body.trim_ascii();
        if body.is_empty() {
            continue;
        }
        if current.subject.is_empty() {
            current
                .subject
                .extend_from_slice(&body[..body.len().min(512)]);
        } else if is_important_log_body_line(body) {
            if !current.important.is_empty() && current.important.len() + 2 < 768 {
                current.important.extend_from_slice(b"; ");
            }
            let room = 768usize.saturating_sub(current.important.len());
            current
                .important
                .extend_from_slice(&body[..body.len().min(room)]);
        }
    }
    flush_compact_commit(&mut output, &current);
    if !output.is_empty() {
        output.push(b'\n');
    }
    output
}

fn flush_compact_commit(output: &mut Vec<u8>, commit: &CompactCommit) {
    let (Some(sha7), false) = (commit.sha7, commit.subject.is_empty()) else {
        return;
    };
    if !output.is_empty() {
        output.push(b'\n');
    }
    output.extend_from_slice(&sha7);
    output.push(b' ');
    output.extend_from_slice(&commit.subject);
    if !commit.important.is_empty() {
        output.extend_from_slice(b" [");
        output.extend_from_slice(&commit.important);
        output.push(b']');
    }
}

fn is_important_log_body_line(line: &[u8]) -> bool {
    [
        b"Refs:".as_slice(),
        b"Ref:",
        b"Fixes:",
        b"Closes:",
        b"Resolves:",
        b"Related:",
        b"Issue:",
        b"BREAKING CHANGE",
        b"Co-authored-by:",
        b"Signed-off-by:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

struct StatLine<'a> {
    raw: &'a [u8],
    parent: &'a [u8],
    insertions: usize,
    deletions: usize,
    keep_raw: bool,
}

#[derive(Default)]
struct StatCommit<'a> {
    header: CompactCommit,
    lines: Vec<StatLine<'a>>,
    summary: Option<&'a [u8]>,
}

pub(super) fn apply_log_stat_compact(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut current = StatCommit::default();
    for line in input.split(|byte| *byte == b'\n') {
        if is_commit_line(line) {
            flush_stat_commit(&mut output, &current);
            current = StatCommit::default();
            current.header.sha7 = line
                .get(b"commit ".len()..b"commit ".len() + 7)
                .and_then(|sha| sha.try_into().ok());
            continue;
        }
        if current.header.sha7.is_none() {
            continue;
        }
        let trimmed = line.trim_ascii();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(body) = line.strip_prefix(b"    ") {
            let body = body.trim_ascii();
            if body.is_empty() {
                continue;
            }
            if current.header.subject.is_empty() {
                current
                    .header
                    .subject
                    .extend_from_slice(&body[..body.len().min(512)]);
            } else if is_important_log_body_line(body) {
                if !current.header.important.is_empty() && current.header.important.len() + 2 < 768
                {
                    current.header.important.extend_from_slice(b"; ");
                }
                let room = 768usize.saturating_sub(current.header.important.len());
                current
                    .header
                    .important
                    .extend_from_slice(&body[..body.len().min(room)]);
            }
            continue;
        }
        if is_stat_summary_line(trimmed) {
            current.summary = Some(trimmed);
        } else if let Some(stat) = parse_stat_line(trimmed) {
            current.lines.push(stat);
        }
    }
    flush_stat_commit(&mut output, &current);
    if !output.is_empty() {
        output.push(b'\n');
    }
    output
}

fn flush_stat_commit(output: &mut Vec<u8>, commit: &StatCommit<'_>) {
    if commit.header.sha7.is_none() || commit.header.subject.is_empty() {
        return;
    }
    flush_compact_commit(output, &commit.header);
    if commit.lines.len() <= 5 {
        for line in &commit.lines {
            write_indented_line(output, line.raw);
        }
    } else {
        let mut index = 0;
        while index < commit.lines.len() {
            let line = &commit.lines[index];
            if line.keep_raw {
                write_indented_line(output, line.raw);
                index += 1;
                continue;
            }
            let mut end = index + 1;
            let mut insertions = line.insertions;
            let mut deletions = line.deletions;
            while end < commit.lines.len()
                && !commit.lines[end].keep_raw
                && commit.lines[end].parent == line.parent
            {
                insertions += commit.lines[end].insertions;
                deletions += commit.lines[end].deletions;
                end += 1;
            }
            if end - index >= 3 {
                output.extend_from_slice(b"\n  ");
                if line.parent == b"." {
                    output.extend_from_slice(b"./");
                } else {
                    output.extend_from_slice(line.parent);
                    if !line.parent.ends_with(b"/") {
                        output.push(b'/');
                    }
                }
                output.extend_from_slice(b" (");
                output.extend_from_slice((end - index).to_string().as_bytes());
                output.extend_from_slice(b" files, +");
                output.extend_from_slice(insertions.to_string().as_bytes());
                output.extend_from_slice(b" -");
                output.extend_from_slice(deletions.to_string().as_bytes());
                output.push(b')');
            } else {
                for item in &commit.lines[index..end] {
                    write_indented_line(output, item.raw);
                }
            }
            index = end;
        }
    }
    if let Some(summary) = commit.summary {
        write_indented_line(output, summary);
    }
}

fn write_indented_line(output: &mut Vec<u8>, line: &[u8]) {
    output.extend_from_slice(b"\n  ");
    output.extend_from_slice(line);
}

fn parse_stat_line(line: &[u8]) -> Option<StatLine<'_>> {
    let pipe = line.iter().position(|byte| *byte == b'|')?;
    let path = line[..pipe].trim_ascii();
    let after = line[pipe + 1..].trim_ascii();
    if path.is_empty() || after.is_empty() {
        return None;
    }
    if after.starts_with(b"Bin ") {
        return Some(StatLine {
            raw: line,
            parent: stat_parent_dir(path),
            insertions: 0,
            deletions: 0,
            keep_raw: true,
        });
    }
    if !after[0].is_ascii_digit() || first_number(after).is_none() {
        return None;
    }
    Some(StatLine {
        raw: line,
        parent: stat_parent_dir(path),
        insertions: after.iter().filter(|byte| **byte == b'+').count(),
        deletions: after.iter().filter(|byte| **byte == b'-').count(),
        keep_raw: find_subslice(path, b"=>").is_some()
            || (path.contains(&b'{') && path.contains(&b'}')),
    })
}

fn stat_parent_dir(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or(b".", |slash| if slash == 0 { b"." } else { &path[..slash] })
}

fn is_stat_summary_line(line: &[u8]) -> bool {
    find_subslice(line, b" file changed").is_some()
        || find_subslice(line, b" files changed").is_some()
}
