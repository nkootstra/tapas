pub(super) fn apply_add(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    process_add_stream(stdout, &mut output);
    process_add_stream(stderr, &mut output);
    output
}

fn process_add_stream(input: &[u8], output: &mut Vec<u8>) {
    let mut ignored_block = false;
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            ignored_block = false;
            continue;
        }
        if line.starts_with(b"The following paths are ignored") {
            ignored_block = true;
            continue;
        }
        if ignored_block && line.starts_with(b"  (") {
            continue;
        }
        if ignored_block && let Some(path) = line.strip_prefix(b"\t") {
            output.extend_from_slice(b"! ");
            output.extend_from_slice(path);
            output.push(b'\n');
            continue;
        }
        if let Some(rest) = line.strip_prefix(b"fatal: pathspec '") {
            if let Some(end) = rest.iter().position(|byte| *byte == b'\'') {
                output.extend_from_slice(b"! ");
                output.extend_from_slice(&rest[..end]);
                output.push(b'\n');
            } else {
                output.extend_from_slice(line);
                output.push(b'\n');
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix(b"warning: CRLF will be replaced by LF in ") {
            output.extend_from_slice(b"! ");
            output.extend_from_slice(rest.strip_suffix(b".").unwrap_or(rest));
            output.push(b'\n');
            continue;
        }
        if line.starts_with(b"The file will have its original line endings") {
            continue;
        }
        output.extend_from_slice(line);
        output.push(b'\n');
    }
}

pub(super) fn apply_checkout(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for raw in stderr.split(|byte| *byte == b'\n') {
        let line = raw.trim_ascii();
        if line.starts_with(b"Switched to") {
            if let Some(open) = line.iter().position(|byte| *byte == b'\'') {
                let after = &line[open + 1..];
                if let Some(close) = after.iter().position(|byte| *byte == b'\'') {
                    output.extend_from_slice(b"^ ");
                    output.extend_from_slice(&after[..close]);
                    output.push(b'\n');
                }
            }
        } else if let Some(rest) = line.strip_prefix(b"HEAD is now at ") {
            let split = rest
                .iter()
                .position(|byte| *byte == b' ')
                .unwrap_or(rest.len());
            output.extend_from_slice(b"^ detached ");
            output.extend_from_slice(&rest[..split.min(7)]);
            let subject = rest[split..].trim_ascii();
            if !subject.is_empty() {
                output.push(b' ');
                output.extend_from_slice(subject);
            }
            output.push(b'\n');
        } else if let Some(rest) = line.strip_prefix(b"Your branch is up to date with '")
            && let Some(close) = rest.iter().position(|byte| *byte == b'\'')
        {
            output.extend_from_slice(b"= ");
            output.extend_from_slice(&rest[..close]);
            output.push(b'\n');
        }
    }
    for raw in stdout.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.len() >= 2 && line[1] == b'\t' {
            output.push(if line[0] == b'D' { b'd' } else { line[0] });
            output.push(b' ');
            output.extend_from_slice(&line[2..]);
            output.push(b'\n');
        }
    }
    output
}

pub(super) fn apply_rebase(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    if stdout.is_empty() {
        scan_rebase(stderr, &mut output);
    } else {
        scan_rebase(stdout, &mut output);
        if !stderr.is_empty() {
            scan_rebase(stderr, &mut output);
        }
    }
    output
}

fn scan_rebase(input: &[u8], output: &mut Vec<u8>) {
    let mut index = 0;
    while index < input.len() {
        if matches!(input[index], b'\r' | b'\n') {
            index += 1;
            continue;
        }
        let line_end = input[index..]
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .map_or(input.len(), |end| index + end);
        let line = &input[index..line_end];
        if let Some(branch) = line.strip_prefix(b"Successfully rebased and updated ") {
            let branch = branch.strip_suffix(b".").unwrap_or(branch);
            let branch = branch.strip_prefix(b"refs/heads/").unwrap_or(branch);
            output.extend_from_slice(b"@ rebased ");
            output.extend_from_slice(branch);
            output.push(b'\n');
        } else if let Some(rest) = line.strip_prefix(b"Current branch ") {
            if find_subslice(rest, b" is up to date").is_some() {
                output.extend_from_slice(b"@ up-to-date\n");
            }
        } else if let Some(subject) = line.strip_prefix(b"Applying: ") {
            output.extend_from_slice(b"r ");
            output.extend_from_slice(subject);
            output.push(b'\n');
        } else if line.starts_with(b"CONFLICT (") {
            emit_conflict(output, line);
        }
        index = line_end;
    }
}

pub(super) fn apply_stash(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let source = if stdout.is_empty() { stderr } else { stdout };
    let trimmed = source.trim_ascii_start();
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    if trimmed.starts_with(b"Saved working directory") {
        if let Some(line) = source
            .split(|byte| *byte == b'\n')
            .map(<[u8]>::trim_ascii)
            .find(|line| !line.is_empty())
        {
            write_stash_save(line, &mut output);
        }
    } else if trimmed.starts_with(b"stash@{") {
        for line in source
            .split(|byte| *byte == b'\n')
            .map(<[u8]>::trim_ascii)
            .filter(|line| line.starts_with(b"stash@{"))
        {
            write_stash_list(line, &mut output);
        }
    } else {
        output.extend_from_slice(stdout);
        output.extend_from_slice(stderr);
    }
    output
}

fn write_stash_save(line: &[u8], output: &mut Vec<u8>) {
    let branch_start = find_subslice(line, b" WIP on ")
        .map(|position| position + b" WIP on ".len())
        .or_else(|| find_subslice(line, b" On ").map(|position| position + b" On ".len()));
    let Some(branch_start) = branch_start else {
        output.extend_from_slice(line);
        output.push(b'\n');
        return;
    };
    write_stash_body(b"$", &line[branch_start..], output);
}

fn write_stash_list(line: &[u8], output: &mut Vec<u8>) {
    let Some(open) = line.iter().position(|byte| *byte == b'{') else {
        return;
    };
    let Some(relative_close) = line[open..].iter().position(|byte| *byte == b'}') else {
        return;
    };
    let close = open + relative_close;
    let mut prefix = b"$".to_vec();
    prefix.extend_from_slice(&line[open + 1..close]);
    let body = line[close + 1..]
        .strip_prefix(b": ")
        .unwrap_or(&line[close + 1..]);
    let after_on = body
        .strip_prefix(b"WIP on ")
        .or_else(|| body.strip_prefix(b"On "));
    if let Some(after_on) = after_on {
        write_stash_body(&prefix, after_on, output);
    } else {
        output.extend_from_slice(&prefix);
        output.push(b' ');
        output.extend_from_slice(body);
        output.push(b'\n');
    }
}

fn write_stash_body(prefix: &[u8], after_on: &[u8], output: &mut Vec<u8>) {
    output.extend_from_slice(prefix);
    output.push(b' ');
    if let Some(colon) = after_on.iter().position(|byte| *byte == b':') {
        output.extend_from_slice(&after_on[..colon]);
        let remainder = after_on[colon + 1..].trim_ascii_start();
        if !remainder.is_empty() {
            output.push(b' ');
            output.extend_from_slice(remainder);
        }
    } else {
        output.extend_from_slice(after_on);
    }
    output.push(b'\n');
}

pub(super) fn apply_status_short(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous_xy = [0, 0];
    let mut previous_dir: &[u8] = b"";
    let mut run_len = 0;
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if !is_short_status_line(line) {
            output.extend_from_slice(line);
            output.push(b'\n');
            run_len = 0;
            continue;
        }
        let xy = [line[0], line[1]];
        let path = &line[3..];
        let rename = matches!(xy[0], b'R' | b'C');
        let dir = if rename { b"" } else { parent_dir(path) };
        let same_run =
            run_len > 0 && !rename && !dir.is_empty() && xy == previous_xy && dir == previous_dir;
        output.extend_from_slice(&xy);
        output.push(b' ');
        if same_run {
            output.extend_from_slice(&path[dir.len()..]);
            run_len += 1;
        } else {
            output.extend_from_slice(path);
            previous_xy = xy;
            previous_dir = dir;
            run_len = usize::from(!rename);
        }
        output.push(b'\n');
    }
    output
}

fn is_short_status_line(line: &[u8]) -> bool {
    line.len() >= 4
        && line[2] == b' '
        && matches!(
            line[0],
            b' ' | b'M' | b'A' | b'D' | b'R' | b'C' | b'U' | b'?' | b'!' | b'T'
        )
        && matches!(
            line[1],
            b' ' | b'M' | b'A' | b'D' | b'R' | b'C' | b'U' | b'?' | b'!' | b'T'
        )
}

use super::find_subslice;
use super::merge::emit_conflict;
use super::status::parent_dir;
