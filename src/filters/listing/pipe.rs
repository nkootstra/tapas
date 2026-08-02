pub(super) fn matches_tree(input: &[u8]) -> bool {
    if input.is_empty() || matches!(input[0], b' ' | b'\t' | 0..=0x1f) {
        return false;
    }
    input
        .split(|byte| *byte == b'\n')
        .take(6)
        .any(|line| unicode_tree_line(line) || parse_ascii_tree_line(line).is_some())
}

pub(super) fn matches_ls_long(input: &[u8]) -> bool {
    input
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
        .is_some_and(|line| is_ls_total(line) || is_ls_long_line(line))
}

fn is_ls_total(line: &[u8]) -> bool {
    line.strip_prefix(b"total ")
        .is_some_and(|rest| !rest.is_empty() && rest.iter().all(u8::is_ascii_digit))
}

fn is_ls_long_line(line: &[u8]) -> bool {
    line.len() >= 10
        && matches!(line[0], b'd' | b'-' | b'l' | b'c' | b'b' | b'p' | b's')
        && line[1..10]
            .iter()
            .all(|byte| matches!(byte, b'r' | b'w' | b'x' | b'-' | b's' | b'S' | b't' | b'T'))
}

pub(super) fn apply_ls_long(input: &[u8]) -> Option<Vec<u8>> {
    if input.is_empty() {
        return Some(Vec::new());
    }
    let mut output = Vec::with_capacity(input.len());
    let mut had_content = false;
    let mut parsed_any = false;
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if is_ls_total(line) {
            had_content = true;
            continue;
        }
        if !is_ls_long_line(line) {
            return None;
        }
        had_content = true;
        let Some(name) = field_remainder(line, 8) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        parsed_any = true;
        if matches!(name, b"." | b"..") {
            continue;
        }
        output.extend_from_slice(name);
        if line[0] == b'd' {
            output.push(b'/');
        }
        output.push(b'\n');
    }
    (parsed_any || !had_content).then_some(output)
}

fn field_remainder(mut line: &[u8], fields: usize) -> Option<&[u8]> {
    for _ in 0..fields {
        line = trim_ascii_start_space(line);
        let field_end = line.iter().position(|byte| matches!(byte, b' ' | b'\t'))?;
        line = &line[field_end..];
    }
    line = trim_ascii_start_space(line);
    let line = trim_ascii_end_space(line);
    (!line.is_empty()).then_some(line)
}

fn trim_ascii_start_space(mut input: &[u8]) -> &[u8] {
    while input
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        input = &input[1..];
    }
    input
}

pub(super) fn trim_ascii_end_space(mut input: &[u8]) -> &[u8] {
    while input
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[..input.len() - 1];
    }
    input
}

pub(super) fn matches_find_ls(input: &[u8]) -> bool {
    let mut saw_any = false;
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if !is_find_ls_line(line) {
            return false;
        }
        saw_any = true;
    }
    saw_any
}

fn is_find_ls_line(line: &[u8]) -> bool {
    let line = trim_ascii_start_space(line);
    let inode_end = line
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(line.len());
    inode_end > 0
        && line
            .get(inode_end)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        && field_remainder(line, 10).is_some()
}

#[derive(Clone, Copy)]
struct FindEntry<'a> {
    path: &'a [u8],
    parent: &'a [u8],
    is_dir: bool,
}

pub(super) fn apply_find_ls(input: &[u8]) -> Vec<u8> {
    let mut entries = Vec::new();
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() || !is_find_ls_line(line) {
            continue;
        }
        let Some(path) = field_remainder(line, 10) else {
            continue;
        };
        entries.push(FindEntry {
            path,
            parent: parent_dir(path),
            is_dir: nth_field(line, 2).is_some_and(|mode| mode.starts_with(b"d")),
        });
    }
    entries.sort_by(|left, right| {
        left.parent
            .cmp(right.parent)
            .then_with(|| left.path.cmp(right.path))
    });

    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < entries.len() {
        let mut end = index + 1;
        while end < entries.len() && entries[end].parent == entries[index].parent {
            end += 1;
        }
        let group = &entries[index..end];
        if group.len() >= 3 {
            output.extend_from_slice(group[0].parent);
            output.extend_from_slice(b"/ (");
            output.extend_from_slice(group.len().to_string().as_bytes());
            output.extend_from_slice(b" entries: ");
            for (position, entry) in group.iter().take(3).enumerate() {
                if position > 0 {
                    output.extend_from_slice(b", ");
                }
                output.extend_from_slice(basename(entry.path));
                if entry.is_dir {
                    output.push(b'/');
                }
            }
            write_omission(&mut output, group.len(), 3);
            output.extend_from_slice(b")\n");
        } else {
            for entry in group {
                output.extend_from_slice(entry.path);
                if entry.is_dir {
                    output.push(b'/');
                }
                output.push(b'\n');
            }
        }
        index = end;
    }
    output
}

fn nth_field(mut input: &[u8], wanted: usize) -> Option<&[u8]> {
    for index in 0..=wanted {
        input = trim_ascii_start_space(input);
        if input.is_empty() {
            return None;
        }
        let end = input
            .iter()
            .position(|byte| matches!(byte, b' ' | b'\t'))
            .unwrap_or(input.len());
        if index == wanted {
            return Some(&input[..end]);
        }
        input = &input[end..];
    }
    None
}

pub(super) fn parent_dir(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or(b".", |index| if index == 0 { b"/" } else { &path[..index] })
}

pub(super) fn basename(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or(path, |index| {
            let tail = &path[index + 1..];
            if tail.is_empty() { path } else { tail }
        })
}

pub(super) fn write_omission(output: &mut Vec<u8>, total: usize, shown: usize) {
    if total <= shown {
        return;
    }
    output.extend_from_slice(b"; ");
    output.extend_from_slice((total - shown).to_string().as_bytes());
    output.extend_from_slice(b" omitted; --raw for all");
}
use super::tree_pipe::{parse_ascii_tree_line, unicode_tree_line};
