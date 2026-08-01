use std::collections::HashSet;

use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterOutput, find_subslice, rfind_subslice,
    strip_ansi,
};

pub fn matches(input: &[u8]) -> bool {
    matches_status(input)
        || matches_branch(input)
        || matches_reflog(input)
        || matches_show(input)
        || matches_diff(input)
        || matches_log(input)
        || matches_commit(input)
        || matches_merge(input)
        || matches_blame(input)
}

pub fn apply_matched(input: &[u8]) -> Result<FilterOutput, FilterError> {
    if matches_status(input) {
        return Ok(FilterOutput::new(
            apply_status(input),
            EvidenceClass::FactComplete,
        ));
    }
    if matches_branch(input) {
        return Ok(FilterOutput::new(
            apply_branch(input),
            EvidenceClass::FactComplete,
        ));
    }
    if matches_reflog(input) {
        return Ok(FilterOutput::new(
            apply_reflog(input),
            EvidenceClass::FactComplete,
        ));
    }
    if matches_show(input) {
        return Ok(FilterOutput::new(
            apply_show(input),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if matches_diff(input) {
        return Ok(FilterOutput::new(
            apply_diff(input),
            EvidenceClass::FactComplete,
        ));
    }
    if matches_log(input) {
        return Ok(FilterOutput::new(
            apply_log_compact(input),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if matches_commit(input) {
        return Ok(FilterOutput::new(
            apply_commit(input),
            EvidenceClass::FactComplete,
        ));
    }
    if matches_merge(input) {
        return Ok(FilterOutput::new(
            apply_merge(input, b""),
            EvidenceClass::FactComplete,
        ));
    }
    if matches_blame(input) {
        return Ok(FilterOutput::new(
            apply_blame(input),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    Err(FilterError::InvalidInput)
}

pub fn dispatch_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    _stderr: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<FilterOutput, FilterError> {
    if argv.len() < 2 {
        return Err(FilterError::InvalidInput);
    }
    if lossless || exit_code != 0 {
        return Ok(passthrough(stdout));
    }

    match argv[1] {
        b"status" => {
            let args = &argv[1..];
            if has_arg(args, b"--porcelain") || has_arg(args, b"-z") {
                Ok(passthrough(stdout))
            } else if has_arg(args, b"--short") || has_arg(args, b"-s") {
                Ok(FilterOutput::new(
                    apply_status_short(stdout),
                    EvidenceClass::FactComplete,
                ))
            } else {
                Ok(FilterOutput::new(
                    apply_status(stdout),
                    EvidenceClass::FactComplete,
                ))
            }
        }
        b"diff" => {
            let args = &argv[1..];
            if [
                b"--stat".as_slice(),
                b"--shortstat",
                b"--name-only",
                b"--name-status",
                b"--compact-summary",
                b"--summary",
                b"--patch-with-stat",
            ]
            .iter()
            .any(|argument| has_arg(args, argument))
            {
                Ok(passthrough(stdout))
            } else {
                Ok(FilterOutput::new(
                    apply_diff(stdout),
                    EvidenceClass::FactComplete,
                ))
            }
        }
        b"log" => {
            let args = &argv[1..];
            let custom = [
                b"--oneline".as_slice(),
                b"--name-only",
                b"--name-status",
                b"--compact-summary",
                b"--no-walk",
                b"--abbrev-commit",
                b"--graph",
                b"-p",
                b"--patch",
                b"-u",
            ]
            .iter()
            .any(|argument| has_arg(args, argument))
                || has_format_or_pretty_arg(args);
            if custom {
                Ok(passthrough(stdout))
            } else if has_arg(args, b"--stat") || has_arg(args, b"--shortstat") {
                Ok(FilterOutput::new(
                    apply_log_stat_compact(stdout),
                    EvidenceClass::PotentiallyLossy,
                ))
            } else {
                Ok(FilterOutput::new(
                    apply_log_compact(stdout),
                    EvidenceClass::PotentiallyLossy,
                ))
            }
        }
        b"show" => {
            let args = &argv[1..];
            let summary = [
                b"--name-only".as_slice(),
                b"--name-status",
                b"--compact-summary",
                b"--no-patch",
                b"--raw",
                b"-s",
            ]
            .iter()
            .any(|argument| has_arg(args, argument));
            let blob = argv[2..]
                .iter()
                .any(|argument| !argument.starts_with(b"-") && argument.contains(&b':'));
            if summary || has_format_or_pretty_arg(args) || blob {
                Ok(passthrough(stdout))
            } else if has_arg(args, b"--stat") || has_arg(args, b"--shortstat") {
                Ok(FilterOutput::new(
                    apply_log_stat_compact(stdout),
                    EvidenceClass::PotentiallyLossy,
                ))
            } else {
                Ok(FilterOutput::new(
                    apply_show(stdout),
                    EvidenceClass::PotentiallyLossy,
                ))
            }
        }
        b"branch" => Ok(FilterOutput::new(
            apply_branch(stdout),
            EvidenceClass::FactComplete,
        )),
        b"reflog" => {
            if has_format_or_pretty_arg(&argv[1..]) || !matches_reflog(stdout) {
                Ok(passthrough(stdout))
            } else {
                Ok(FilterOutput::new(
                    apply_reflog(stdout),
                    EvidenceClass::FactComplete,
                ))
            }
        }
        b"commit" => Ok(FilterOutput::new(
            apply_commit(stdout),
            EvidenceClass::FactComplete,
        )),
        b"merge" => Ok(FilterOutput::new(
            apply_merge(stdout, b""),
            EvidenceClass::FactComplete,
        )),
        b"blame" => {
            let args = &argv[1..];
            let alternate = [
                b"-s".as_slice(),
                b"--porcelain",
                b"-p",
                b"--line-porcelain",
                b"--incremental",
                b"-e",
                b"--show-email",
            ]
            .iter()
            .any(|argument| has_arg(args, argument));
            if alternate {
                Ok(passthrough(stdout))
            } else {
                Ok(FilterOutput::new(
                    apply_blame(stdout),
                    EvidenceClass::PotentiallyLossy,
                ))
            }
        }
        b"add" => Ok(FilterOutput::new(
            apply_add(stdout, b""),
            EvidenceClass::FactComplete,
        )),
        b"checkout" | b"switch" => Ok(FilterOutput::new(
            apply_checkout(stdout, b""),
            EvidenceClass::FactComplete,
        )),
        b"fetch" => Ok(FilterOutput::new(
            apply_fetch(b""),
            EvidenceClass::FactComplete,
        )),
        b"pull" => Ok(FilterOutput::new(
            apply_pull(stdout, b""),
            EvidenceClass::FactComplete,
        )),
        b"push" => Ok(FilterOutput::new(
            apply_push(b""),
            EvidenceClass::FactComplete,
        )),
        b"rebase" => Ok(FilterOutput::new(
            apply_rebase(stdout, b""),
            EvidenceClass::FactComplete,
        )),
        b"stash" => Ok(FilterOutput::new(
            apply_stash(stdout, b""),
            EvidenceClass::FactComplete,
        )),
        _ => Ok(passthrough(stdout)),
    }
}

/// Apply the pinned Git wrapper semantics when the caller owns both streams.
/// Successful argv-only helpers intentionally render retained stderr facts on
/// stdout, matching smll; lossless, failed, and bypassed commands retain their
/// original stream ownership.
pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    if argv.len() < 2 {
        return Err(FilterError::InvalidInput);
    }
    if lossless || exit_code != 0 {
        return Ok(StreamFilterOutput::new(
            stdout.to_vec(),
            stderr.to_vec(),
            EvidenceClass::ByteExact,
        ));
    }

    let compact = match argv[1] {
        b"add" => Some(apply_add(stdout, stderr)),
        b"checkout" | b"switch" => Some(apply_checkout(stdout, stderr)),
        b"fetch" => Some(apply_fetch(stderr)),
        b"pull" => Some(apply_pull(stdout, stderr)),
        b"push" => Some(apply_push(stderr)),
        b"merge" => Some(apply_merge(stdout, stderr)),
        b"rebase" => Some(apply_rebase(stdout, stderr)),
        b"stash" => Some(apply_stash(stdout, stderr)),
        _ => None,
    };
    if let Some(stdout) = compact {
        return Ok(StreamFilterOutput::new(
            stdout,
            Vec::new(),
            EvidenceClass::FactComplete,
        ));
    }

    let filtered = dispatch_argv(argv, stdout, stderr, exit_code, lossless)?;
    let preserved_stderr = if filtered.evidence == EvidenceClass::ByteExact {
        stderr.to_vec()
    } else {
        Vec::new()
    };
    Ok(StreamFilterOutput::new(
        filtered.bytes,
        preserved_stderr,
        filtered.evidence,
    ))
}

fn apply_add(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

fn apply_checkout(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

fn apply_fetch(stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stderr.len());
    process_ref_stderr(stderr, &mut output, b'<', b"From ", b"-> FETCH_HEAD");
    output
}

fn apply_push(stderr: &[u8]) -> Vec<u8> {
    if find_subslice(stderr, b"Everything up-to-date").is_some() {
        return b"= up-to-date\n".to_vec();
    }
    let mut output = Vec::with_capacity(stderr.len());
    process_ref_stderr(stderr, &mut output, b'>', b"To ", b"");
    output
}

fn apply_pull(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

fn process_ref_stderr(
    stderr: &[u8],
    output: &mut Vec<u8>,
    sigil: u8,
    skip_prefix: &[u8],
    skip_needle: &[u8],
) {
    for line in stderr.split(|byte| *byte == b'\n') {
        if line.is_empty()
            || (!skip_prefix.is_empty() && line.starts_with(skip_prefix))
            || is_git_progress_line(line)
            || (!skip_needle.is_empty() && find_subslice(line, skip_needle).is_some())
        {
            continue;
        }
        if write_bracket_ref(line, output) {
            continue;
        }
        let line = line.trim_ascii_start();
        if is_ref_update_line(line) {
            write_ref_update(line, output, sigil);
        }
    }
}

fn is_git_progress_line(line: &[u8]) -> bool {
    line.starts_with(b"remote")
        || line.starts_with(b"Counting")
        || line.starts_with(b"Compressing")
        || line.starts_with(b"Receiving")
        || line.starts_with(b"Resolving")
        || line.starts_with(b"Writing objects")
        || line.starts_with(b"Total ")
        || line.starts_with(b"Delta ")
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

fn apply_rebase(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

fn apply_stash(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

fn apply_status_short(input: &[u8]) -> Vec<u8> {
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

fn passthrough(input: &[u8]) -> FilterOutput {
    FilterOutput::new(input.to_vec(), EvidenceClass::ByteExact)
}

fn has_arg(argv: &[&[u8]], expected: &[u8]) -> bool {
    argv.contains(&expected)
}

fn has_format_or_pretty_arg(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        matches!(*argument, b"--format" | b"--pretty")
            || argument.starts_with(b"--format=")
            || argument.starts_with(b"--pretty=")
    })
}

fn matches_diff(input: &[u8]) -> bool {
    find_diff_start(input).is_some()
}

fn matches_branch(input: &[u8]) -> bool {
    first_nonempty_line(input).is_some_and(|line| {
        line.len() >= 3
            && line[2] != b' '
            && ((line[0] == b'*' && line[1] == b' ') || (line[0] == b' ' && line[1] == b' '))
    })
}

fn matches_reflog(input: &[u8]) -> bool {
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

fn apply_reflog(input: &[u8]) -> Vec<u8> {
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

fn apply_branch(input: &[u8]) -> Vec<u8> {
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

fn matches_log(input: &[u8]) -> bool {
    first_nonempty_line(input).is_some_and(is_commit_line)
}

fn matches_show(input: &[u8]) -> bool {
    matches_log(input) && find_diff_start(&input[..input.len().min(8 * 1024)]).is_some()
}

fn apply_show(input: &[u8]) -> Vec<u8> {
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

fn find_diff_start(input: &[u8]) -> Option<usize> {
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
        if sha7.is_none() {
            continue;
        }
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
            output.extend_from_slice(&sha7.unwrap());
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

fn apply_log_compact(input: &[u8]) -> Vec<u8> {
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

fn apply_log_stat_compact(input: &[u8]) -> Vec<u8> {
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

fn matches_commit(input: &[u8]) -> bool {
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

fn apply_commit(input: &[u8]) -> Vec<u8> {
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

fn skip_mode_number(input: &[u8]) -> &[u8] {
    input
        .iter()
        .position(|byte| *byte == b' ')
        .map_or(input, |space| &input[space + 1..])
}

fn write_summary(output: &mut Vec<u8>, line: &[u8]) {
    output.push(b'+');
    output.extend_from_slice(number_before_marker(line, b" insertion").unwrap_or(b"0"));
    output.extend_from_slice(b"/-");
    output.extend_from_slice(number_before_marker(line, b" deletion").unwrap_or(b"0"));
    output.extend_from_slice(b" files=");
    output.extend_from_slice(first_number(line).unwrap_or(b"0"));
    output.push(b'\n');
}

fn first_number(input: &[u8]) -> Option<&[u8]> {
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

fn matches_merge(input: &[u8]) -> bool {
    first_nonempty_line(input).is_some_and(|line| {
        line.starts_with(b"Merge made by")
            || (line.starts_with(b"Updating ") && find_subslice(line, b"..").is_some())
            || line.starts_with(b"Already up to date")
    })
}

fn apply_merge(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

fn emit_conflict(output: &mut Vec<u8>, line: &[u8]) {
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

fn matches_blame(input: &[u8]) -> bool {
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

fn apply_blame(input: &[u8]) -> Vec<u8> {
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
        let sha7: [u8; 7] = sha[..7].try_into().unwrap();
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

fn matches_status(input: &[u8]) -> bool {
    first_nonempty_line(input).is_some_and(|line| {
        line.starts_with(b"On branch ")
            || line.starts_with(b"HEAD detached ")
            || is_operation_state(line)
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum StatusSection {
    None,
    Staged,
    Unstaged,
    Untracked,
    Unmerged,
}

#[derive(Clone, Copy)]
struct StatusEntry<'a> {
    code: &'static [u8],
    path: &'a [u8],
}

fn apply_status(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut section = StatusSection::None;
    let mut branch = Vec::new();
    let mut branch_written = false;
    let mut ahead = None;
    let mut behind = None;
    let mut upstream = None;
    let mut run: Vec<(StatusSection, &[u8])> = Vec::new();
    let mut run_key: &[u8] = b"";
    let mut run_dir: &[u8] = b"";

    for line in input.split(|byte| *byte == b'\n') {
        if !branch_written {
            if let Some(name) = line.strip_prefix(b"On branch ") {
                branch.extend_from_slice(&name[..name.len().min(256)]);
                continue;
            }
            if let Some(reference) = line.strip_prefix(b"HEAD detached at ") {
                branch.extend_from_slice(b"HEAD:");
                let room = 256usize.saturating_sub(branch.len());
                branch.extend_from_slice(&reference[..reference.len().min(room)]);
                continue;
            }
            if line.starts_with(b"interactive rebase in progress") {
                branch.clear();
                branch.extend_from_slice(b"rebase-in-progress");
            } else if let Some(rest) = line.strip_prefix(b"Your branch is ahead") {
                ahead = count_after(rest, b"by ");
                continue;
            } else if let Some(rest) = line.strip_prefix(b"Your branch is behind") {
                behind = count_after(rest, b"by ");
                continue;
            } else if let Some(rest) = line.strip_prefix(b"Your branch is up to date with '") {
                upstream = rest
                    .iter()
                    .position(|byte| *byte == b'\'')
                    .map(|end| &rest[..end]);
                continue;
            } else if line.starts_with(b"Your branch and") {
                if let Some((a, b)) = diverged_counts(line) {
                    ahead = Some(a);
                    behind = Some(b);
                }
                continue;
            }
        }

        if is_operation_state(line) {
            if !branch_written {
                if branch.is_empty() {
                    branch.extend_from_slice(b"operation-in-progress");
                }
                write_branch_line(&mut output, &branch, ahead, behind, upstream);
                branch_written = true;
            }
            flush_status_run(&mut output, &run, run_dir);
            run.clear();
            run_key = b"";
            output.extend_from_slice(b"! ");
            output.extend_from_slice(line.trim_ascii());
            output.push(b'\n');
            continue;
        }

        match line {
            b"Changes to be committed:" => {
                section = StatusSection::Staged;
                continue;
            }
            b"Changes not staged for commit:" => {
                section = StatusSection::Unstaged;
                continue;
            }
            b"Untracked files:" => {
                section = StatusSection::Untracked;
                continue;
            }
            b"Unmerged paths:" => {
                section = StatusSection::Unmerged;
                continue;
            }
            _ => {}
        }

        if is_status_hint(line)
            || line.trim_ascii().is_empty()
            || line.starts_with(b"no changes added to commit")
            || line.starts_with(b"nothing to commit")
            || line.starts_with(b"nothing added to commit")
            || line.starts_with(b"You have unmerged paths")
            || line.starts_with(b"All conflicts fixed")
        {
            continue;
        }

        if let Some(content) = line.strip_prefix(b"\t") {
            if !branch_written {
                write_branch_line(&mut output, &branch, ahead, behind, upstream);
                branch_written = true;
            }
            if let Some(entry) = status_entry(section, content) {
                let dir = parent_dir(entry.path);
                let key = status_group_key(section, entry.code);
                if !dir.is_empty() && !run.is_empty() && key == run_key && dir == run_dir {
                    if run.len() == 64 {
                        flush_status_run(&mut output, &run, run_dir);
                        run.clear();
                    }
                    run.push((section, content));
                } else {
                    flush_status_run(&mut output, &run, run_dir);
                    run.clear();
                    run.push((section, content));
                    run_key = key;
                    run_dir = dir;
                }
            } else {
                flush_status_run(&mut output, &run, run_dir);
                run.clear();
                run_key = b"";
                write_status_entry(&mut output, section, content);
            }
            continue;
        }

        if branch_written {
            flush_status_run(&mut output, &run, run_dir);
            run.clear();
            run_key = b"";
            output.extend_from_slice(line);
            output.push(b'\n');
        }
    }

    flush_status_run(&mut output, &run, run_dir);
    if !branch_written && !branch.is_empty() {
        write_branch_line(&mut output, &branch, ahead, behind, upstream);
    }
    output
}

fn status_entry(section: StatusSection, content: &[u8]) -> Option<StatusEntry<'_>> {
    let prefixes: &[(&[u8], &[u8], bool)] = match section {
        StatusSection::Staged => &[
            (b"modified:   ", b"S", false),
            (b"new file:   ", b"A", false),
            (b"deleted:    ", b"D", false),
            (b"renamed:    ", b"R", false),
        ],
        StatusSection::Unstaged => &[
            (b"new file:   ", b"I", false),
            (b"modified:   ", b"M", false),
            (b"deleted:    ", b"d", false),
        ],
        StatusSection::Unmerged => &[
            (b"both modified:   ", b"UU", false),
            (b"added by us:    ", b"AU", false),
            (b"added by them:  ", b"UA", false),
            (b"deleted by us:  ", b"DU", false),
            (b"deleted by them:", b"UD", true),
        ],
        StatusSection::Untracked => {
            return Some(StatusEntry {
                code: b"?",
                path: content,
            });
        }
        StatusSection::None => return None,
    };
    for &(prefix, code, trim_start) in prefixes {
        if let Some(path) = content.strip_prefix(prefix) {
            return Some(StatusEntry {
                code,
                path: if trim_start {
                    path.trim_ascii_start()
                } else {
                    path
                },
            });
        }
    }
    Some(StatusEntry {
        code: match section {
            StatusSection::Staged => b"S",
            StatusSection::Unstaged => b"M",
            StatusSection::Unmerged => b"UU",
            _ => unreachable!(),
        },
        path: content,
    })
}

fn status_group_key(section: StatusSection, code: &[u8]) -> &[u8] {
    if section == StatusSection::Unmerged {
        b"U"
    } else {
        code
    }
}

fn flush_status_run(output: &mut Vec<u8>, run: &[(StatusSection, &[u8])], dir: &[u8]) {
    if run.is_empty() {
        return;
    }
    if write_numeric_status_range(output, run, dir) {
        return;
    }
    if run.len() >= 3 && !dir.is_empty() {
        for (index, &(section, content)) in run.iter().enumerate() {
            let entry = status_entry(section, content).unwrap();
            output.extend_from_slice(entry.code);
            output.push(b' ');
            if index > 0 && entry.path.starts_with(dir) && entry.path.len() > dir.len() {
                output.extend_from_slice(&entry.path[dir.len()..]);
            } else {
                output.extend_from_slice(entry.path);
            }
            output.push(b'\n');
        }
    } else {
        for &(section, content) in run {
            write_status_entry(output, section, content);
        }
    }
}

fn write_numeric_status_range(
    output: &mut Vec<u8>,
    run: &[(StatusSection, &[u8])],
    dir: &[u8],
) -> bool {
    if run.len() < 6 || dir.is_empty() {
        return false;
    }
    let first_entry = status_entry(run[0].0, run[0].1).unwrap();
    let Some(first) = numeric_basename(first_entry.path, dir) else {
        return false;
    };
    let mut last = first;
    for (offset, &(section, content)) in run.iter().enumerate().skip(1) {
        let entry = status_entry(section, content).unwrap();
        let Some(parsed) = numeric_basename(entry.path, dir) else {
            return false;
        };
        if entry.code != first_entry.code
            || parsed.prefix != first.prefix
            || parsed.suffix != first.suffix
            || parsed.value != first.value.saturating_add(offset)
        {
            return false;
        }
        last = parsed;
    }
    output.extend_from_slice(first_entry.code);
    output.push(b' ');
    output.extend_from_slice(dir);
    output.extend_from_slice(first.basename);
    output.extend_from_slice(b"..");
    output.extend_from_slice(last.basename);
    output.push(b'\n');
    true
}

#[derive(Clone, Copy)]
struct NumericBasename<'a> {
    basename: &'a [u8],
    prefix: &'a [u8],
    suffix: &'a [u8],
    value: usize,
}

fn numeric_basename<'a>(path: &'a [u8], dir: &[u8]) -> Option<NumericBasename<'a>> {
    let basename = path.strip_prefix(dir)?;
    if basename.is_empty() {
        return None;
    }
    let end = basename.iter().rposition(u8::is_ascii_digit)? + 1;
    let start = basename[..end]
        .iter()
        .rposition(|byte| !byte.is_ascii_digit())
        .map_or(0, |position| position + 1);
    let value = basename[start..end]
        .iter()
        .try_fold(0usize, |value, byte| {
            value
                .checked_mul(10)?
                .checked_add(usize::from(*byte - b'0'))
        })?;
    Some(NumericBasename {
        basename,
        prefix: &basename[..start],
        suffix: &basename[end..],
        value,
    })
}

fn write_status_entry(output: &mut Vec<u8>, section: StatusSection, content: &[u8]) {
    if let Some(entry) = status_entry(section, content) {
        output.extend_from_slice(entry.code);
        output.push(b' ');
        output.extend_from_slice(entry.path);
    } else {
        output.extend_from_slice(content);
    }
    output.push(b'\n');
}

fn write_branch_line(
    output: &mut Vec<u8>,
    branch: &[u8],
    ahead: Option<&[u8]>,
    behind: Option<&[u8]>,
    upstream: Option<&[u8]>,
) {
    output.extend_from_slice(b"# ");
    output.extend_from_slice(branch);
    if let Some(value) = ahead {
        output.extend_from_slice(b" +");
        output.extend_from_slice(value);
    }
    if let Some(value) = behind {
        output.extend_from_slice(b" -");
        output.extend_from_slice(value);
    }
    if ahead.is_none()
        && behind.is_none()
        && let Some(value) = upstream
    {
        output.extend_from_slice(b" =");
        output.extend_from_slice(value);
    }
    output.push(b'\n');
}

fn is_status_hint(line: &[u8]) -> bool {
    line.starts_with(b"  (") && line.ends_with(b")")
}

fn is_operation_state(line: &[u8]) -> bool {
    let line = line.trim_ascii();
    line.starts_with(b"interactive rebase in progress")
        || line.starts_with(b"All conflicts fixed but you are still merging")
        || line.starts_with(b"You have unmerged paths")
        || line.starts_with(b"You are currently ")
}

fn count_after<'a>(input: &'a [u8], marker: &[u8]) -> Option<&'a [u8]> {
    let start = find_subslice(input, marker)? + marker.len();
    let end = input[start..]
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(input.len() - start);
    (end > 0).then_some(&input[start..start + end])
}

fn diverged_counts(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let start = find_subslice(line, b"and have ")? + b"and have ".len();
    let rest = &line[start..];
    let ahead_end = rest.iter().position(|byte| !byte.is_ascii_digit())?;
    let second_start = ahead_end + find_subslice(&rest[ahead_end..], b" and ")? + b" and ".len();
    let behind_end = rest[second_start..]
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(rest.len() - second_start);
    (ahead_end > 0 && behind_end > 0).then_some((
        &rest[..ahead_end],
        &rest[second_start..second_start + behind_end],
    ))
}

fn parent_dir(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or(b"", |position| &path[..=position])
}

fn apply_diff(input: &[u8]) -> Vec<u8> {
    let had_trailing_newline = input.ends_with(b"\n");
    let content = input.strip_suffix(b"\n").unwrap_or(input);
    let mut output = Vec::with_capacity(input.len());
    let mut first = true;

    for raw in content.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.as_slice();
        let transformed = if line.starts_with(b"diff --git a/") {
            let rest = &line[b"diff --git a/".len()..];
            let mut value = b"d ".to_vec();
            if let Some(split) = rfind_subslice(rest, b" b/") {
                let old = &rest[..split];
                let new = &rest[split + 3..];
                value.extend_from_slice(old);
                if old != new {
                    value.extend_from_slice(b" -> ");
                    value.extend_from_slice(new);
                }
            } else {
                value.extend_from_slice(rest);
            }
            Some(value)
        } else if line.starts_with(b"@@ ") {
            Some(compact_hunk_header(line))
        } else if is_diff_metadata(line) {
            None
        } else if line.starts_with(b"Binary files ") && line.ends_with(b" differ") {
            Some(b"B".to_vec())
        } else {
            Some(line.to_vec())
        };

        if let Some(line) = transformed {
            if !first {
                output.push(b'\n');
            }
            output.extend_from_slice(&line);
            first = false;
        }
    }

    if had_trailing_newline && !first {
        output.push(b'\n');
    }
    output
}

fn compact_hunk_header(line: &[u8]) -> Vec<u8> {
    let after_open = &line[3..];
    let Some(close) = find_subslice(after_open, b" @@") else {
        let mut output = b"@ ".to_vec();
        output.extend_from_slice(after_open);
        return output;
    };
    let coords = &after_open[..close];
    let context = &after_open[close + 3..];
    let mut output = b"@".to_vec();
    if let Some(split) = find_subslice(coords, b" +") {
        output.extend_from_slice(
            coords[..split]
                .strip_prefix(b"-")
                .unwrap_or(&coords[..split]),
        );
        output.push(b'|');
        output.extend_from_slice(&coords[split + 2..]);
    } else {
        output.extend_from_slice(coords);
    }
    output.extend_from_slice(context);
    output
}

fn is_diff_metadata(line: &[u8]) -> bool {
    [
        b"index ".as_slice(),
        b"similarity index ",
        b"dissimilarity index ",
        b"--- a/",
        b"--- /dev/null",
        b"+++ b/",
        b"+++ /dev/null",
        b"rename from ",
        b"rename to ",
        b"copy from ",
        b"copy to ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn first_nonempty_line(input: &[u8]) -> Option<&[u8]> {
    input
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
}
