use super::*;

pub(super) fn passthrough(input: &[u8]) -> FilterOutput {
    FilterOutput::new(input.to_vec(), EvidenceClass::ByteExact)
}

pub(super) fn has_arg(argv: &[&[u8]], expected: &[u8]) -> bool {
    argv.contains(&expected)
}

pub(super) fn has_format_or_pretty_arg(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        matches!(*argument, b"--format" | b"--pretty")
            || argument.starts_with(b"--format=")
            || argument.starts_with(b"--pretty=")
    })
}

pub(super) fn matches_diff(input: &[u8]) -> bool {
    find_diff_start(input).is_some()
}

pub(super) fn matches_branch(input: &[u8]) -> bool {
    first_nonempty_line(input).is_some_and(|line| {
        line.len() >= 3
            && line[2] != b' '
            && ((line[0] == b'*' && line[1] == b' ') || (line[0] == b' ' && line[1] == b' '))
    })
}

pub(super) fn matches_reflog(input: &[u8]) -> bool {
    first_nonempty_line(input)
        .and_then(parse_reflog_line)
        .is_some()
}

struct ReflogLine<'a> {
    sha: [u8; 7],
    reference: &'a [u8],
    index: &'a [u8],
    rest: &'a [u8],
}

fn parse_reflog_line(line: &[u8]) -> Option<ReflogLine<'_>> {
    if line.len() < 16 || !line[..7].iter().all(u8::is_ascii_hexdigit) || line[7] != b' ' {
        return None;
    }
    let after_sha = &line[8..];
    let at_brace = find_subslice(after_sha, b"@{")?;
    if at_brace == 0 || after_sha[..at_brace].contains(&b' ') {
        return None;
    }
    let after_brace = &after_sha[at_brace + 2..];
    let close = after_brace.iter().position(|byte| *byte == b'}')?;
    let index = &after_brace[..close];
    if index.is_empty() || !index.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let rest = after_brace[close + 1..].strip_prefix(b": ")?;
    Some(ReflogLine {
        sha: line[..7].try_into().ok()?,
        reference: &after_sha[..at_brace],
        index,
        rest,
    })
}

pub(super) fn apply_reflog(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous = None;
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Some(parsed) = parse_reflog_line(line) else {
            output.extend_from_slice(line);
            output.push(b'\n');
            previous = None;
            continue;
        };
        if previous == Some(parsed.sha) {
            output.push(b'~');
        } else {
            output.extend_from_slice(&parsed.sha);
        }
        output.push(b' ');
        if parsed.reference != b"HEAD" {
            output.extend_from_slice(parsed.reference);
        }
        output.push(b'@');
        output.extend_from_slice(parsed.index);
        output.push(b' ');
        output.extend_from_slice(parsed.rest);
        output.push(b'\n');
        previous = Some(parsed.sha);
    }
    output
}

pub(super) fn apply_branch(input: &[u8]) -> Vec<u8> {
    let has_remotes = find_subslice(input, b"remotes/").is_some();
    let mut locals = HashSet::new();
    let mut origins = HashSet::new();
    if has_remotes {
        for raw in input.split(|byte| *byte == b'\n') {
            let line = raw.strip_suffix(b"\r").unwrap_or(raw);
            let Some(token) = first_branch_token(line) else {
                continue;
            };
            if let Some(name) = token.strip_prefix(b"remotes/origin/") {
                if name != b"HEAD" {
                    origins.insert(name);
                }
            } else if !token.starts_with(b"remotes/") {
                locals.insert(token);
            }
        }
    }

    let mut output = Vec::with_capacity(input.len());
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        let mut marker: &[u8] = b"";
        if has_remotes && let Some(token) = first_branch_token(line) {
            if let Some(name) = token.strip_prefix(b"remotes/origin/") {
                if name != b"HEAD" && locals.contains(name) {
                    continue;
                }
            } else if !token.starts_with(b"remotes/") && origins.contains(token) {
                marker = b" =o";
            }
        }
        if write_verbose_branch_line(&mut output, line, marker) {
            continue;
        }
        if let Some(name) = line.strip_prefix(b"* ") {
            output.extend_from_slice(b"* ");
            output.extend_from_slice(name);
            output.extend_from_slice(marker);
            output.push(b'\n');
        } else if line.starts_with(b"  ") {
            output.push(b' ');
            output.extend_from_slice(line.trim_ascii_start());
            output.extend_from_slice(marker);
            output.push(b'\n');
        } else {
            output.extend_from_slice(line);
            output.push(b'\n');
        }
    }
    output
}

fn first_branch_token(line: &[u8]) -> Option<&[u8]> {
    let rest = if let Some(rest) = line.strip_prefix(b"* ") {
        rest
    } else if line.starts_with(b" ") {
        line.trim_ascii_start()
    } else {
        return None;
    };
    let end = rest
        .iter()
        .position(|byte| *byte == b' ')
        .unwrap_or(rest.len());
    (end > 0).then_some(&rest[..end])
}

fn write_verbose_branch_line(output: &mut Vec<u8>, line: &[u8], marker: &[u8]) -> bool {
    let current = line.starts_with(b"* ");
    if !current && !line.starts_with(b"  ") {
        return false;
    }
    let rest = line[2..].trim_ascii_start();
    let Some(sha_start) = find_sha7_token(rest) else {
        return false;
    };
    if sha_start == 0 {
        return false;
    }
    let branch = rest[..sha_start].trim_ascii_end();
    let after_branch = rest[sha_start..].trim_ascii_start();
    let sha = &after_branch[..7];
    let tail = after_branch[7..].trim_ascii_start();
    if current {
        output.extend_from_slice(b"* ");
    }
    output.extend_from_slice(branch);
    output.push(b' ');
    output.extend_from_slice(sha);
    if !tail.is_empty() {
        output.push(b' ');
        write_compact_verbose_tail(output, tail);
    }
    output.extend_from_slice(marker);
    output.push(b'\n');
    true
}

fn find_sha7_token(line: &[u8]) -> Option<usize> {
    if line.len() < 7 {
        return None;
    }
    (0..=line.len().saturating_sub(7)).find(|&index| {
        (index == 0 || line[index - 1] == b' ')
            && (index + 7 == line.len() || line[index + 7] == b' ')
            && line[index..index + 7].iter().all(u8::is_ascii_hexdigit)
    })
}

fn write_compact_verbose_tail(output: &mut Vec<u8>, tail: &[u8]) {
    let Some(rest) = tail.strip_prefix(b"[") else {
        output.extend_from_slice(tail);
        return;
    };
    let Some(end) = rest.iter().position(|byte| *byte == b']') else {
        output.extend_from_slice(tail);
        return;
    };
    let upstream = &rest[..end];
    output.push(b'@');
    if let Some(upstream) = upstream.strip_suffix(b": gone") {
        output.extend_from_slice(upstream);
        output.extend_from_slice(b" gone");
    } else {
        output.extend_from_slice(upstream);
    }
    let subject = rest[end + 1..].trim_ascii_start();
    if !subject.is_empty() {
        output.push(b' ');
        output.extend_from_slice(subject);
    }
}
