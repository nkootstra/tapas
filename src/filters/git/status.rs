pub(super) fn matches_status(input: &[u8]) -> bool {
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

pub(super) fn apply_status(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut section = StatusSection::None;
    let mut branch = Vec::new();
    let mut branch_written = false;
    let mut declared_clean = false;
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
            } else if line.starts_with(b"Your branch and") || line.starts_with(b"and have ") {
                // Git wraps divergence across two lines, keeping the counts on the
                // continuation: "Your branch and 'origin/main' have diverged,\nand have 1
                // and 2 different commits each, respectively." Only the continuation
                // carries them, so both openings have to be offered to the parser.
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
                write_branch_line(&mut output, &branch, ahead, behind, upstream, false);
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

        if declares_clean_tree(line) {
            declared_clean = true;
            continue;
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
                write_branch_line(&mut output, &branch, ahead, behind, upstream, false);
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
        // A bare branch line is indistinguishable from a truncated listing, since a dirty
        // tree opens with the same line. Mark the clean case so it reads as a complete
        // answer -- but only when git said the tree was clean, never merely because no
        // entries were parsed. Absence is not evidence here: piped input can be cut off
        // mid-listing, and claiming clean over that would be a false FactComplete.
        write_branch_line(
            &mut output,
            &branch,
            ahead,
            behind,
            upstream,
            declared_clean,
        );
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
            let Some(entry) = status_entry(section, content) else {
                write_status_entry(output, section, content);
                continue;
            };
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
    let Some(first_entry) = status_entry(run[0].0, run[0].1) else {
        return false;
    };
    let Some(first) = numeric_basename(first_entry.path, dir) else {
        return false;
    };
    let mut last = first;
    for (offset, &(section, content)) in run.iter().enumerate().skip(1) {
        let Some(entry) = status_entry(section, content) else {
            return false;
        };
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
    clean: bool,
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
    if clean {
        output.extend_from_slice(b" (clean)");
    }
    output.push(b'\n');
}

fn is_status_hint(line: &[u8]) -> bool {
    line.starts_with(b"  (") && line.ends_with(b")")
}

/// Git's own assertion that the working tree holds nothing to report.
///
/// Deliberately narrow: the other `nothing to commit` variants qualify themselves --
/// `-uno` emits `nothing to commit (use -u to show untracked files)` while untracked
/// files exist, and a repo without commits emits `nothing to commit (create/copy files
/// ...)`. Those stay unmarked rather than overstate what git claimed.
///
/// The wording is versioned: `working tree` since 2.9, `working directory` before it, and
/// a parenthesized form before that. The parenthesized form is belt-and-braces -- git old
/// enough to emit it also prefixes the branch line with `# `, which `matches_status`
/// declines, so the document never reaches this filter -- but the sentence is unambiguous
/// and costs nothing to accept.
fn declares_clean_tree(line: &[u8]) -> bool {
    line.starts_with(b"nothing to commit, working tree clean")
        || line.starts_with(b"nothing to commit, working directory clean")
        || line.starts_with(b"nothing to commit (working directory clean)")
}

/// Openings of the state notices `git status` prints above the file listing.
///
/// `You are currently ` covers most of them -- bisecting, cherry-picking, reverting,
/// rebasing, editing and splitting during a rebase. The rest are spelled differently and
/// have to be listed. Missing one is not cosmetic: the notice is dropped from a clean
/// tree's output entirely, and the tree then reports as `(clean)` while the operation is
/// still outstanding.
const OPERATION_STATES: &[&[u8]] = &[
    b"interactive rebase in progress",
    b"All conflicts fixed but you are still merging",
    b"You have unmerged paths",
    b"You are currently ",
    b"You are in the middle of an am session",
    b"The current patch is empty",
    b"Cherry-pick currently in progress",
    b"Revert currently in progress",
    b"You are editing the todo file",
    b"You are not currently on a branch",
    // Not outstanding work, but the tree is only partly present -- reporting it as simply
    // clean omits the reason most of the repository is missing.
    b"You are in a sparse checkout",
];

fn is_operation_state(line: &[u8]) -> bool {
    let line = line.trim_ascii();
    OPERATION_STATES
        .iter()
        .any(|opening| line.starts_with(opening))
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

pub(super) fn parent_dir(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or(b"", |position| &path[..=position])
}
use super::diff::first_nonempty_line;
use super::find_subslice;
