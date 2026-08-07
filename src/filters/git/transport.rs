use super::commit::write_summary;
use super::find_subslice;

pub(super) fn apply_fetch(stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stderr.len());
    process_ref_stderr(stderr, &mut output, b'<', b"From ", b"-> FETCH_HEAD");
    output
}

pub(super) fn apply_push(stderr: &[u8]) -> Vec<u8> {
    if find_subslice(stderr, b"Everything up-to-date").is_some() {
        return b"= up-to-date\n".to_vec();
    }
    let mut output = Vec::with_capacity(stderr.len());
    process_ref_stderr(stderr, &mut output, b'>', b"To ", b"");
    output
}

pub(super) fn compact_push_stdout(stdout: &[u8]) -> Option<Vec<u8>> {
    if std::str::from_utf8(stdout).is_err() {
        return None;
    }
    for line in stdout.split(|byte| *byte == b'\n') {
        if line.trim_ascii().is_empty() {
            continue;
        }
        if !is_tracking_boilerplate(line) {
            return None;
        }
    }
    Some(Vec::new())
}

pub(super) fn compact_push_stderr(stderr: &[u8]) -> Option<Vec<u8>> {
    if std::str::from_utf8(stderr).is_err() {
        return None;
    }
    if find_subslice(stderr, b"Everything up-to-date").is_some() {
        let mut saw_up_to_date = false;
        for line in stderr.split(|byte| *byte == b'\n') {
            let line = line.trim_ascii();
            if line.is_empty() || is_git_progress_line(line) || line.starts_with(b"To ") {
                continue;
            }
            if line == b"Everything up-to-date" && !saw_up_to_date {
                saw_up_to_date = true;
                continue;
            }
            return None;
        }
        return saw_up_to_date.then(|| b"= up-to-date\n".to_vec());
    }
    try_process_ref_stderr(stderr, b'>', b"To ", b"")
}

fn is_tracking_boilerplate(line: &[u8]) -> bool {
    let Some(rest) = line.strip_prefix(b"branch '") else {
        return false;
    };
    let Some(quote) = rest.iter().position(|byte| *byte == b'\'') else {
        return false;
    };
    let branch = &rest[..quote];
    let Some(remote) = rest[quote + 1..].strip_prefix(b" set up to track 'origin/") else {
        return false;
    };
    !branch.is_empty() && remote.strip_suffix(b"'.") == Some(branch)
}

pub(super) fn apply_pull(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    process_ref_stderr(stderr, &mut output, b'<', b"From ", b"-> FETCH_HEAD");
    if find_subslice(stdout, b"Already up to date.").is_some() {
        output.extend_from_slice(b"@ up-to-date\n");
        return output;
    }

    let mut updating_range = None;
    let mut fast_forward = false;
    let mut merge_commit = false;
    let mut summary = None;
    for line in stdout.split(|byte| *byte == b'\n') {
        if let Some(range) = line.strip_prefix(b"Updating ") {
            updating_range = Some(range);
        } else if line == b"Fast-forward" || line.starts_with(b"Fast forward") {
            fast_forward = true;
        } else if line.starts_with(b"Merge made by") {
            merge_commit = true;
        } else if find_subslice(line, b" file").is_some()
            && find_subslice(line, b" changed").is_some()
        {
            summary = Some(line);
        }
    }
    if fast_forward {
        output.extend_from_slice(b"@ fast-forward");
        if let Some(range) = updating_range {
            output.push(b' ');
            output.extend_from_slice(range);
        }
        output.push(b'\n');
    } else if merge_commit {
        output.extend_from_slice(b"@ merge-commit");
        if let Some(range) = updating_range {
            output.push(b' ');
            output.extend_from_slice(range);
        }
        output.push(b'\n');
    }
    if let Some(summary) = summary {
        write_summary(&mut output, summary.trim_ascii_start());
    }
    output
}

pub(super) fn compact_pull_stdout(stdout: &[u8]) -> Option<Vec<u8>> {
    if std::str::from_utf8(stdout).is_err() {
        return None;
    }
    let mut saw_outcome = false;
    for raw in stdout.split(|byte| *byte == b'\n') {
        let line = raw.trim_ascii();
        if line.is_empty() {
            continue;
        }
        let recognized = if let Some(range) = line.strip_prefix(b"Updating ") {
            is_oid_range(range)
        } else if line == b"Fast-forward"
            || line.starts_with(b"Fast forward")
            || line.starts_with(b"Merge made by")
            || line == b"Already up to date."
        {
            saw_outcome = true;
            true
        } else {
            (find_subslice(line, b" file").is_some() && find_subslice(line, b" changed").is_some())
                || find_subslice(line, b" | ").is_some()
                || line.starts_with(b"create mode ")
                || line.starts_with(b"delete mode ")
                || line.starts_with(b"rename ")
                || line.starts_with(b"copy ")
                || line.starts_with(b"mode change ")
        };
        if !recognized {
            return None;
        }
    }
    saw_outcome.then(|| apply_pull(stdout, b""))
}

pub(super) fn compact_pull_stderr(stderr: &[u8]) -> Option<Vec<u8>> {
    if std::str::from_utf8(stderr).is_err() {
        return None;
    }
    try_process_ref_stderr(stderr, b'<', b"From ", b"-> FETCH_HEAD")
}

fn is_oid_range(range: &[u8]) -> bool {
    let Some(dots) = find_subslice(range, b"..") else {
        return false;
    };
    dots >= 4
        && range[..dots].iter().all(u8::is_ascii_hexdigit)
        && range[dots + 2..].len() >= 4
        && range[dots + 2..].iter().all(u8::is_ascii_hexdigit)
}

fn process_ref_stderr(
    stderr: &[u8],
    output: &mut Vec<u8>,
    sigil: u8,
    skip_prefix: &[u8],
    skip_needle: &[u8],
) {
    let _ = try_process_ref_stderr_into(stderr, output, sigil, skip_prefix, skip_needle);
}

fn try_process_ref_stderr(
    stderr: &[u8],
    sigil: u8,
    skip_prefix: &[u8],
    skip_needle: &[u8],
) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    try_process_ref_stderr_into(stderr, &mut output, sigil, skip_prefix, skip_needle)
        .then_some(output)
}

fn try_process_ref_stderr_into(
    stderr: &[u8],
    output: &mut Vec<u8>,
    sigil: u8,
    skip_prefix: &[u8],
    skip_needle: &[u8],
) -> bool {
    for line in stderr.split(|byte| *byte == b'\n') {
        let trimmed = line.trim_ascii();
        if trimmed.is_empty()
            || (!skip_prefix.is_empty() && line.starts_with(skip_prefix))
            || is_git_progress_line(trimmed)
            || (!skip_needle.is_empty() && find_subslice(trimmed, skip_needle).is_some())
        {
            continue;
        }
        if write_bracket_ref(line, output) {
            continue;
        }
        if is_ref_update_line(trimmed) {
            write_ref_update(trimmed, output, sigil);
            continue;
        }
        return false;
    }
    true
}

fn is_git_progress_line(line: &[u8]) -> bool {
    line.starts_with(b"remote: Enumerating objects:")
        || line.starts_with(b"remote: Counting objects:")
        || line.starts_with(b"remote: Compressing objects:")
        || line.starts_with(b"remote: Finding sources:")
        || line.starts_with(b"remote: Getting sizes:")
        || line.starts_with(b"remote: Resolving deltas:")
        || line.starts_with(b"remote: Total ")
        || line.starts_with(b"Counting objects:")
        || line.starts_with(b"Compressing objects:")
        || line.starts_with(b"Receiving objects:")
        || line.starts_with(b"Resolving deltas:")
        || line.starts_with(b"Writing objects:")
        || line.starts_with(b"Total ")
        || line.starts_with(b"Delta compression using ")
}

fn write_bracket_ref(line: &[u8], output: &mut Vec<u8>) -> bool {
    let kind: &[u8] = if find_subslice(line, b"[new branch]").is_some()
        || find_subslice(line, b"[new tag]").is_some()
    {
        b"+ new ".as_slice()
    } else if find_subslice(line, b"[deleted]").is_some() {
        b"- deleted "
    } else if find_subslice(line, b"[rejected]").is_some() {
        b"! rejected "
    } else {
        return false;
    };
    let Some(close) = line.iter().position(|byte| *byte == b']') else {
        return false;
    };
    let rest = line[close + 1..].trim_ascii();
    if !rest.is_empty() {
        output.extend_from_slice(kind);
        output.extend_from_slice(rest);
        output.push(b'\n');
    }
    true
}

fn is_ref_update_line(line: &[u8]) -> bool {
    let Some(dots) = find_subslice(&line[..line.len().min(84)], b"..") else {
        return false;
    };
    dots >= 4 && line[..dots].iter().all(u8::is_ascii_hexdigit)
}

fn write_ref_update(line: &[u8], output: &mut Vec<u8>, sigil: u8) {
    let Some(dots) = find_subslice(line, b"..") else {
        return;
    };
    let after = &line[dots + 2..];
    let sha_end = after
        .iter()
        .position(|byte| *byte == b' ')
        .unwrap_or(after.len());
    let mut rest = after[sha_end..].trim_ascii();
    if let Some(arrow) = find_subslice(rest, b" -> ")
        && rest[..arrow] == rest[arrow + 4..]
    {
        rest = &rest[..arrow];
    }
    output.push(sigil);
    output.push(b' ');
    output.extend_from_slice(&line[..dots.min(7)]);
    output.extend_from_slice(b"..");
    output.extend_from_slice(&after[..sha_end.min(7)]);
    if !rest.is_empty() {
        output.push(b' ');
        output.extend_from_slice(rest);
    }
    output.push(b'\n');
}
