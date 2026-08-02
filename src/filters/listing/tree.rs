#[derive(Clone, Copy)]
struct TreeEntry<'a> {
    depth: usize,
    name: &'a [u8],
    is_dir: bool,
}

fn parse_tree_line(line: &[u8]) -> Option<(usize, &[u8], bool)> {
    if let Some((depth, name)) = parse_ascii_tree_line(line) {
        return Some((depth, name, name.ends_with(b"/")));
    }
    let prefix_len = tree_prefix_len(line);
    let name = line[prefix_len..].trim_ascii();
    (!name.is_empty()).then_some((tree_depth(&line[..prefix_len]), name, name.ends_with(b"/")))
}

fn is_tree_summary(line: &[u8]) -> bool {
    (find_subslice(line, b" directory").is_some() || find_subslice(line, b" directories").is_some())
        && find_subslice(line, b" file").is_some()
}

pub(super) fn apply_tree_compact(input: &[u8]) -> Option<Vec<u8>> {
    let mut entries = Vec::new();
    let mut summary = None;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        if is_tree_summary(line) {
            summary = Some(line);
            continue;
        }
        let (depth, mut name, dir_hint) = parse_tree_line(line)?;
        if name.ends_with(b"/") {
            name = &name[..name.len() - 1];
        }
        entries.push(TreeEntry {
            depth,
            name,
            is_dir: dir_hint || depth == 0,
        });
    }
    if entries.is_empty() {
        return summary.map(|line| {
            let mut output = line.to_vec();
            output.push(b'\n');
            output
        });
    }
    for index in 0..entries.len().saturating_sub(1) {
        if entries[index + 1].depth > entries[index].depth {
            entries[index].is_dir = true;
        }
    }
    let mut output = Vec::with_capacity(input.len());
    emit_tree_entry(&entries, 0, &mut output);
    if let Some(line) = summary {
        write_output_line(&mut output, line);
    }
    output.push(b'\n');
    Some(output)
}

fn emit_tree_entry(entries: &[TreeEntry<'_>], index: usize, output: &mut Vec<u8>) {
    let entry = entries[index];
    write_tree_line(output, entry.depth, entry.name, entry.is_dir);
    if !entry.is_dir {
        return;
    }
    let end = tree_subtree_end(entries, index);
    let child_depth = entry.depth + 1;
    let file_count = entries[index + 1..end]
        .iter()
        .filter(|child| child.depth == child_depth && !child.is_dir)
        .count();
    let mut file_group_emitted = false;
    let mut child = index + 1;
    while child < end {
        if entries[child].depth != child_depth {
            child += 1;
            continue;
        }
        let child_entry = entries[child];
        if !child_entry.is_dir {
            if file_count >= 4 {
                if !file_group_emitted {
                    write_collapsed_files(output, entries, index, end, file_count);
                    file_group_emitted = true;
                }
            } else {
                write_tree_line(output, child_entry.depth, child_entry.name, false);
            }
            child += 1;
            continue;
        }
        let child_end = tree_subtree_end(entries, child);
        let direct_count = entries[child + 1..child_end]
            .iter()
            .filter(|candidate| candidate.depth == child_entry.depth + 1)
            .count();
        let all_files = direct_children_all_files(entries, child, child_end);
        if direct_count >= 4 && (child_entry.depth >= 2 || all_files) {
            write_collapsed_dir(output, entries, child, child_end, direct_count, all_files);
        } else {
            emit_tree_entry(entries, child, output);
        }
        child = child_end;
    }
}

fn tree_subtree_end(entries: &[TreeEntry<'_>], index: usize) -> usize {
    let depth = entries[index].depth;
    let mut end = index + 1;
    while end < entries.len() && entries[end].depth > depth {
        end += 1;
    }
    end
}

fn direct_children_all_files(entries: &[TreeEntry<'_>], index: usize, end: usize) -> bool {
    let child_depth = entries[index].depth + 1;
    let mut saw = false;
    for entry in &entries[index + 1..end] {
        if entry.depth != child_depth {
            continue;
        }
        saw = true;
        if entry.is_dir {
            return false;
        }
    }
    saw
}

fn write_collapsed_files(
    output: &mut Vec<u8>,
    entries: &[TreeEntry<'_>],
    index: usize,
    end: usize,
    count: usize,
) {
    start_output_line(output);
    write_indent(output, entries[index].depth + 1);
    output.push(b'(');
    output.extend_from_slice(count.to_string().as_bytes());
    output.extend_from_slice(b" files: ");
    let child_depth = entries[index].depth + 1;
    let mut shown = 0;
    for entry in &entries[index + 1..end] {
        if entry.depth != child_depth || entry.is_dir || shown == 3 {
            continue;
        }
        if shown > 0 {
            output.extend_from_slice(b", ");
        }
        output.extend_from_slice(entry.name);
        shown += 1;
    }
    write_omission(output, count, 3);
    output.push(b')');
}

fn write_collapsed_dir(
    output: &mut Vec<u8>,
    entries: &[TreeEntry<'_>],
    index: usize,
    end: usize,
    count: usize,
    all_files: bool,
) {
    let entry = entries[index];
    start_output_line(output);
    write_indent(output, entry.depth);
    output.extend_from_slice(entry.name);
    if !entry.name.ends_with(b"/") {
        output.push(b'/');
    }
    output.extend_from_slice(b" (");
    output.extend_from_slice(count.to_string().as_bytes());
    output.extend_from_slice(if all_files {
        b" files: "
    } else {
        b" entries: "
    });
    let child_depth = entry.depth + 1;
    let mut shown = 0;
    for child in &entries[index + 1..end] {
        if child.depth != child_depth || shown == 3 {
            continue;
        }
        if shown > 0 {
            output.extend_from_slice(b", ");
        }
        output.extend_from_slice(child.name);
        if child.is_dir && !child.name.ends_with(b"/") {
            output.push(b'/');
        }
        shown += 1;
    }
    write_omission(output, count, 3);
    output.push(b')');
}

fn write_tree_line(output: &mut Vec<u8>, depth: usize, name: &[u8], is_dir: bool) {
    start_output_line(output);
    write_indent(output, depth);
    output.extend_from_slice(name);
    if is_dir && !name.ends_with(b"/") && name != b"." {
        output.push(b'/');
    }
}

pub(super) fn write_output_line(output: &mut Vec<u8>, line: &[u8]) {
    start_output_line(output);
    output.extend_from_slice(line);
}

pub(super) fn start_output_line(output: &mut Vec<u8>) {
    if !output.is_empty() {
        output.push(b'\n');
    }
}

fn write_indent(output: &mut Vec<u8>, depth: usize) {
    for _ in 0..depth {
        output.extend_from_slice(b"  ");
    }
}
use super::find_subslice;
use super::pipe::write_omission;
use super::tree_pipe::{parse_ascii_tree_line, tree_depth, tree_prefix_len};
