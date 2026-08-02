pub(super) fn find_has_type_file(argv: &[&[u8]]) -> bool {
    argv.windows(2)
        .any(|pair| pair[0] == b"-type" && pair[1] == b"f")
}

pub(super) fn matches_find_plain(input: &[u8]) -> bool {
    if input.is_empty() {
        return false;
    }
    let mut saw_any = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        if line.contains(&0) || matches!(line[0], b' ' | b'\t') {
            return false;
        }
        saw_any = true;
    }
    saw_any
}

#[derive(Clone, Copy)]
struct PlainEntry<'a> {
    path: &'a [u8],
    parent: &'a [u8],
}

pub(super) fn apply_find_plain(input: &[u8], files_noun: bool) -> Vec<u8> {
    let mut entries: Vec<_> = input
        .split(|byte| *byte == b'\n')
        .map(|raw| raw.strip_suffix(b"\r").unwrap_or(raw))
        .filter(|path| !path.is_empty() && !path.contains(&0))
        .map(|path| PlainEntry {
            path,
            parent: parent_dir(path),
        })
        .collect();
    entries.sort_by(|left, right| {
        left.parent
            .cmp(right.parent)
            .then_with(|| left.path.cmp(right.path))
    });

    let mut output = Vec::with_capacity(input.len());
    let noun: &[u8] = if files_noun { b"files" } else { b"entries" };
    let mut index = 0;
    while index < entries.len() {
        let mut end = index + 1;
        while end < entries.len() && entries[end].parent == entries[index].parent {
            end += 1;
        }
        let group = &entries[index..end];
        if group.len() >= 3 {
            write_parent_label(&mut output, group[0].parent);
            output.extend_from_slice(b" (");
            output.extend_from_slice(group.len().to_string().as_bytes());
            output.push(b' ');
            output.extend_from_slice(noun);
            output.extend_from_slice(b": ");
            for (position, entry) in group.iter().take(3).enumerate() {
                if position > 0 {
                    output.extend_from_slice(b", ");
                }
                output.extend_from_slice(basename(entry.path));
            }
            write_omission(&mut output, group.len(), 3);
            output.extend_from_slice(b")\n");
        } else {
            for entry in group {
                output.extend_from_slice(entry.path);
                output.push(b'\n');
            }
        }
        index = end;
    }
    output
}

fn write_parent_label(output: &mut Vec<u8>, parent: &[u8]) {
    if parent == b"." {
        output.extend_from_slice(b"./");
        return;
    }
    output.extend_from_slice(parent);
    if !parent.ends_with(b"/") {
        output.push(b'/');
    }
}
use super::pipe::{basename, parent_dir, write_omission};
