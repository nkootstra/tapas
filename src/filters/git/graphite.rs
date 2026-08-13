use super::{EvidenceClass, apply_status, matches_status, strip_ansi};

pub(super) fn route(argv: &[&[u8]], stdout: &[u8]) -> Option<(Vec<u8>, EvidenceClass)> {
    match argv {
        [_, b"log"] => {
            compact_default_log(stdout).map(|bytes| (bytes, EvidenceClass::PotentiallyLossy))
        }
        [_, b"log", b"short"] => {
            compact_short_log(stdout).map(|bytes| (bytes, EvidenceClass::PotentiallyLossy))
        }
        [_, b"log", b"long"] => {
            compact_long_log(stdout).map(|bytes| (bytes, EvidenceClass::PotentiallyLossy))
        }
        [_, b"status"] if valid_git_status(stdout) => {
            Some((apply_status(stdout), EvidenceClass::FactComplete))
        }
        _ => None,
    }
}

fn clean_utf8(input: &[u8]) -> Option<String> {
    let clean = strip_ansi(input);
    String::from_utf8(clean).ok()
}

fn compact_default_log(input: &[u8]) -> Option<Vec<u8>> {
    let clean = clean_utf8(input)?;
    let mut saw_branch = false;
    let mut saw_commit = false;
    let mut output = String::with_capacity(clean.len());
    for line in clean.lines() {
        if line.is_empty() || !valid_graph_line(line, true) {
            return None;
        }
        saw_branch |= line.contains(['◉', '○']);
        let detail = graph_detail(line);
        let commit = detail.is_some_and(is_commit_detail);
        saw_commit |= commit;
        if detail.is_none() || detail.is_some_and(is_relative_time) {
            continue;
        }
        output.push_str(line.trim_end());
        output.push('\n');
    }
    (saw_branch && saw_commit).then_some(output.into_bytes())
}

fn compact_short_log(input: &[u8]) -> Option<Vec<u8>> {
    let clean = clean_utf8(input)?;
    let mut saw_branch = false;
    for line in clean.lines() {
        if line.is_empty() || !valid_graph_line(line, false) {
            return None;
        }
        let branch = line.contains(['◉', '○']);
        saw_branch |= branch;
        if !branch {
            return None;
        }
        if graph_detail(line)
            .is_some_and(|detail| is_commit_detail(detail) || is_relative_time(detail))
        {
            return None;
        }
    }
    saw_branch.then_some(normalize_lines(&clean))
}

fn compact_long_log(input: &[u8]) -> Option<Vec<u8>> {
    let clean = clean_utf8(input)?;
    let mut saw_commit = false;
    for line in clean.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(star) = trimmed.find('*') {
            let detail = trimmed[star + 1..].trim_start();
            if !is_commit_detail(detail) {
                return None;
            }
            saw_commit = true;
        } else if !trimmed.chars().all(is_graph_character) {
            return None;
        }
    }
    saw_commit.then_some(normalize_lines(&clean))
}

fn valid_graph_line(line: &str, require_detail_lines: bool) -> bool {
    let mut saw_graph = false;
    for character in line.chars() {
        if is_graph_character(character) {
            saw_graph = true;
            continue;
        }
        return saw_graph && (!require_detail_lines || !character.is_control());
    }
    saw_graph
}

fn graph_detail(line: &str) -> Option<&str> {
    let detail = line.trim_start_matches(is_graph_character).trim();
    (!detail.is_empty()).then_some(detail)
}

fn is_graph_character(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '◉' | '○'
                | '│'
                | '─'
                | '┌'
                | '┐'
                | '└'
                | '┘'
                | '├'
                | '┤'
                | '┬'
                | '┴'
                | '┼'
                | '╭'
                | '╮'
                | '╯'
                | '╰'
                | '*'
                | '|'
                | '/'
                | '\\'
        )
}

fn is_commit_detail(detail: &str) -> bool {
    let Some((sha, _)) = detail.split_once(" - ") else {
        return false;
    };
    (7..=40).contains(&sha.len()) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_relative_time(detail: &str) -> bool {
    detail.ends_with(" ago")
        && detail
            .split_whitespace()
            .next()
            .is_some_and(|count| count.bytes().all(|byte| byte.is_ascii_digit()))
}

fn normalize_lines(input: &str) -> Vec<u8> {
    let mut output = input
        .lines()
        .flat_map(|line| [line.trim_end().as_bytes(), b"\n"].concat())
        .collect::<Vec<_>>();
    if input.is_empty() {
        output.clear();
    }
    output
}

fn valid_git_status(input: &[u8]) -> bool {
    if std::str::from_utf8(input).is_err() || !matches_status(input) {
        return false;
    }
    input.split(|byte| *byte == b'\n').all(|line| {
        line.is_empty()
            || line.starts_with(b"On branch ")
            || line.starts_with(b"HEAD detached ")
            || line.starts_with(b"Your branch ")
            || line.starts_with(b"Changes to be committed:")
            || line.starts_with(b"Changes not staged for commit:")
            || line.starts_with(b"Untracked files:")
            || line.starts_with(b"Unmerged paths:")
            || line.starts_with(b"interactive rebase in progress")
            || line.starts_with(b"Last commands done")
            || line.starts_with(b"Next commands to do")
            || line.starts_with(b"You are currently ")
            || line.starts_with(b"  (")
            || line.starts_with(b"\t")
            || line.starts_with(b"no changes added to commit")
            || line.starts_with(b"nothing to commit")
            || line.starts_with(b"nothing added to commit")
            || line.starts_with(b"You have unmerged paths")
            || line.starts_with(b"All conflicts fixed")
    })
}
