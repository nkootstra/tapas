use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterOutput, command_basename, find_subslice,
};

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    argv.first()
        .copied()
        .map(command_basename)
        .is_some_and(|command| {
            matches!(
                command,
                b"find" | b"tree" | b"ls" | b"du" | b"wc" | b"env" | b"rg"
            )
        })
}

pub fn matches(input: &[u8]) -> bool {
    matches_tree(input) || matches_ls_long(input) || matches_find_ls(input) || matches_du(input)
}

pub fn apply_matched(input: &[u8]) -> Result<FilterOutput, FilterError> {
    if matches_tree(input) {
        return Ok(FilterOutput::new(
            apply_tree_pipe(input),
            EvidenceClass::FactComplete,
        ));
    }
    if matches_ls_long(input) {
        let bytes = apply_ls_long(input).ok_or(FilterError::InvalidInput)?;
        return Ok(FilterOutput::new(bytes, EvidenceClass::PotentiallyLossy));
    }
    if matches_find_ls(input) {
        return Ok(FilterOutput::new(
            apply_find_ls(input),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if matches_du(input) {
        return Ok(FilterOutput::new(
            apply_du(input, true),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    Err(FilterError::InvalidInput)
}

pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    _exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    if argv.is_empty() {
        return Err(FilterError::InvalidInput);
    }
    if lossless {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }
    let command = command_basename(argv[0]);
    if requests_exact_query(argv) {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }
    if command == b"find" {
        if find_requests_exact(argv) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        if matches_find_plain(stdout) {
            return Ok(StreamFilterOutput::new(
                apply_find_plain(stdout, find_has_type_file(argv)),
                stderr.to_vec(),
                EvidenceClass::PotentiallyLossy,
            ));
        }
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }
    if command == b"tree" {
        if tree_requests_exact(argv) || !matches_tree(stdout) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        let Some(compact) = apply_tree_compact(stdout) else {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        };
        return Ok(StreamFilterOutput::new(
            compact,
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if command == b"ls" {
        if ls_requests_exact(argv) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        let compact = if matches_ls_long(stdout) {
            apply_ls_long(stdout)
        } else {
            apply_ls_plain(stdout, ls_wants_columns(argv))
        };
        let Some(compact) = compact else {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        };
        return Ok(StreamFilterOutput::new(
            compact,
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if command == b"du" {
        if !matches_du(stdout) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        return Ok(StreamFilterOutput::new(
            apply_du(stdout, du_has_summarize(argv)),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if command == b"wc" {
        return Ok(StreamFilterOutput::new(
            apply_wc(stdout, stderr),
            Vec::new(),
            EvidenceClass::FactComplete,
        ));
    }
    if command == b"env" && env_is_listing(argv) {
        return Ok(StreamFilterOutput::new(
            apply_env(stdout, stderr),
            Vec::new(),
            EvidenceClass::PotentiallyLossy,
        ));
    }
    if command == b"rg" {
        if rg_is_file_mode(argv) && matches_rg_files(stdout) {
            return Ok(StreamFilterOutput::new(
                apply_rg_files(stdout),
                stderr.to_vec(),
                EvidenceClass::FactComplete,
            ));
        }
        if rg_requests_exact(argv) {
            return Ok(StreamFilterOutput::passthrough(stdout, stderr));
        }
        if matches_rg_pattern(stdout) {
            return Ok(StreamFilterOutput::new(
                apply_rg_pattern(stdout),
                stderr.to_vec(),
                EvidenceClass::FactComplete,
            ));
        }
    }
    Ok(StreamFilterOutput::passthrough(stdout, stderr))
}

fn find_requests_exact(argv: &[&[u8]]) -> bool {
    argv[1..].iter().any(|argument| {
        matches!(
            *argument,
            b"-ls"
                | b"-fls"
                | b"-printf"
                | b"-fprintf"
                | b"-print0"
                | b"-fprint0"
                | b"-exec"
                | b"-execdir"
                | b"-ok"
                | b"-okdir"
                | b"-delete"
                | b"-D"
        )
    })
}

fn find_has_type_file(argv: &[&[u8]]) -> bool {
    argv.windows(2)
        .any(|pair| pair[0] == b"-type" && pair[1] == b"f")
}

fn matches_find_plain(input: &[u8]) -> bool {
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

fn apply_find_plain(input: &[u8], files_noun: bool) -> Vec<u8> {
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

fn requests_exact_query(argv: &[&[u8]]) -> bool {
    for argument in &argv[1..] {
        if *argument == b"--" {
            break;
        }
        if matches!(*argument, b"--help" | b"--version") {
            return true;
        }
    }
    matches!(argv.get(1), Some(&b"help") | Some(&b"version"))
}

fn tree_requests_exact(argv: &[&[u8]]) -> bool {
    for argument in &argv[1..] {
        if *argument == b"--" {
            break;
        }
        if argument.len() < 2 || argument[0] != b'-' {
            continue;
        }
        if !matches!(
            *argument,
            b"-a" | b"-d" | b"-L" | b"-I" | b"-P" | b"--dirsfirst" | b"--noreport" | b"--prune"
        ) {
            return true;
        }
    }
    false
}

fn ls_requests_exact(argv: &[&[u8]]) -> bool {
    let mut options = true;
    for argument in &argv[1..] {
        if !options {
            continue;
        }
        if *argument == b"--" {
            options = false;
            continue;
        }
        if argument.len() < 2 || argument[0] != b'-' {
            continue;
        }
        if argument[1] == b'-' {
            if matches!(
                *argument,
                b"--all" | b"--almost-all" | b"--directory" | b"--recursive" | b"--classify"
            ) || argument.starts_with(b"--classify=")
                || argument.starts_with(b"--indicator-style=")
                || is_human_ls_format(argument)
            {
                continue;
            }
            return true;
        }
        if argument[1..].iter().any(|flag| {
            !matches!(
                flag,
                b'1' | b'A' | b'C' | b'F' | b'R' | b'a' | b'd' | b'm' | b'p' | b'x'
            )
        }) {
            return true;
        }
    }
    false
}

fn du_has_summarize(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        matches!(*argument, b"-s" | b"--summarize")
            || (argument.len() >= 2
                && argument[0] == b'-'
                && argument[1] != b'-'
                && argument[1..].contains(&b's'))
    })
}

fn apply_wc(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if stdout.is_empty() {
        return stderr.to_vec();
    }
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for raw in stdout.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let line = raw.trim_ascii();
        let mut position = 0;
        let mut counts = 0;
        while position < line.len() && counts < 3 {
            let start = position;
            while line.get(position).is_some_and(u8::is_ascii_digit) {
                position += 1;
            }
            if position == start {
                break;
            }
            if counts > 0 {
                output.push(b' ');
            }
            output.extend_from_slice(&line[start..position]);
            counts += 1;
            while line
                .get(position)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                position += 1;
            }
        }
        if counts == 0 {
            output.extend_from_slice(raw);
        } else if position < line.len() {
            output.push(b' ');
            output.extend_from_slice(&line[position..]);
        }
        output.push(b'\n');
    }
    output.extend_from_slice(stderr);
    output
}

fn env_is_listing(argv: &[&[u8]]) -> bool {
    argv[1..].iter().all(|argument| {
        argument.is_empty()
            || argument[0] == b'-'
            || argument
                .iter()
                .position(|byte| *byte == b'=')
                .is_some_and(|position| position > 0)
    })
}

fn apply_env(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for raw in stdout.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
            output.extend_from_slice(line);
            output.push(b'\n');
            continue;
        };
        let key = &line[..separator];
        let value = &line[separator + 1..];
        output.extend_from_slice(key);
        output.push(b'=');
        if env_sensitive_key(key) {
            if value.len() <= 4 {
                output.extend_from_slice(b"****");
            } else {
                output.extend_from_slice(&value[..2]);
                output.extend_from_slice(b"****");
                output.extend_from_slice(&value[value.len() - 2..]);
            }
        } else if value.len() > 100 {
            output.extend_from_slice(&value[..50]);
            output.extend_from_slice(b"...");
        } else {
            output.extend_from_slice(value);
        }
        output.push(b'\n');
    }
    output.extend_from_slice(stderr);
    output
}

fn env_sensitive_key(key: &[u8]) -> bool {
    [
        b"key".as_slice(),
        b"secret",
        b"password",
        b"token",
        b"credential",
        b"auth",
        b"private",
        b"api_key",
        b"apikey",
        b"access_key",
        b"jwt",
    ]
    .iter()
    .any(|needle| contains_ascii_case_insensitive(key, needle))
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

fn is_human_ls_format(argument: &[u8]) -> bool {
    argument.strip_prefix(b"--format=").is_some_and(|format| {
        matches!(
            format,
            b"across" | b"commas" | b"horizontal" | b"single-column" | b"vertical"
        )
    })
}

fn ls_wants_columns(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        if let Some(format) = argument.strip_prefix(b"--format=") {
            return matches!(format, b"across" | b"commas" | b"horizontal" | b"vertical");
        }
        argument.len() >= 2
            && argument[0] == b'-'
            && argument[1] != b'-'
            && argument[1..]
                .iter()
                .any(|flag| matches!(flag, b'C' | b'x' | b'm'))
    })
}

fn apply_ls_plain(input: &[u8], columns: bool) -> Option<Vec<u8>> {
    if input.is_empty() {
        return Some(Vec::new());
    }
    if ls_looks_like_blocks(input) {
        apply_ls_blocks(input, columns)
    } else {
        Some(apply_ls_flat(input, columns))
    }
}

fn ls_looks_like_blocks(input: &[u8]) -> bool {
    let mut previous_blank = true;
    let mut saw_header = false;
    let mut saw_content = false;
    let mut pending_blank = false;
    let mut saw_interior_blank = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            previous_blank = true;
            if saw_content {
                pending_blank = true;
            }
            continue;
        }
        if pending_blank {
            saw_interior_blank = true;
        }
        pending_blank = false;
        if previous_blank && line.len() >= 2 && line.ends_with(b":") {
            saw_header = true;
        }
        previous_blank = false;
        saw_content = true;
    }
    saw_header && saw_interior_blank
}

fn apply_ls_flat(input: &[u8], columns: bool) -> Vec<u8> {
    let mut names = Vec::new();
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        if columns {
            tokenize_ls_row(line, &mut names);
        } else {
            names.push(line);
        }
    }
    if columns {
        names.sort();
    }
    let mut output = Vec::with_capacity(input.len());
    for name in names {
        if matches!(name, b"." | b"..") {
            continue;
        }
        output.extend_from_slice(name);
        output.push(b'\n');
    }
    output
}

fn tokenize_ls_row<'a>(line: &'a [u8], output: &mut Vec<&'a [u8]>) {
    let mut index = 0;
    let mut start = None;
    while index < line.len() {
        match line[index] {
            b' ' => {
                let mut end = index;
                while line.get(end) == Some(&b' ') {
                    end += 1;
                }
                if end - index >= 2
                    && let Some(token_start) = start.take()
                {
                    push_ls_token(&line[token_start..index], output);
                }
                index = end;
            }
            b'\t' => {
                if let Some(token_start) = start.take() {
                    push_ls_token(&line[token_start..index], output);
                }
                index += 1;
            }
            b',' if index + 1 == line.len() || line.get(index + 1) == Some(&b' ') => {
                if let Some(token_start) = start.take() {
                    push_ls_token(&line[token_start..index], output);
                }
                index += 1;
            }
            _ => {
                start.get_or_insert(index);
                index += 1;
            }
        }
    }
    if let Some(token_start) = start {
        push_ls_token(&line[token_start..], output);
    }
}

fn push_ls_token<'a>(token: &'a [u8], output: &mut Vec<&'a [u8]>) {
    let token = token.trim_ascii();
    if !token.is_empty() {
        output.push(token);
    }
}

fn apply_ls_blocks(input: &[u8], columns: bool) -> Option<Vec<u8>> {
    type Segment<'a> = (Option<&'a [u8]>, Vec<&'a [u8]>);

    let mut header = None;
    let mut entries = Vec::new();
    let mut segments: Vec<Segment<'_>> = Vec::new();
    let mut previous_blank = true;
    for raw in input.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            previous_blank = true;
            continue;
        }
        if previous_blank && line.len() >= 2 && line.ends_with(b":") {
            if columns {
                entries.sort();
            }
            segments.push((header, std::mem::take(&mut entries)));
            header = Some(&line[..line.len() - 1]);
            previous_blank = false;
            continue;
        }
        previous_blank = false;
        if columns {
            tokenize_ls_row(line, &mut entries);
        } else {
            entries.push(line);
        }
    }
    if columns {
        entries.sort();
    }
    segments.push((header, entries));

    let mut output = Vec::with_capacity(input.len());
    for (header, entries) in segments {
        flush_ls_segment(&mut output, header, &entries);
    }
    if output.is_empty() {
        None
    } else {
        output.push(b'\n');
        Some(output)
    }
}

fn flush_ls_segment(output: &mut Vec<u8>, header: Option<&[u8]>, entries: &[&[u8]]) {
    let real: Vec<_> = entries
        .iter()
        .copied()
        .filter(|entry| !matches!(*entry, b"." | b".."))
        .collect();
    if real.is_empty() {
        return;
    }
    let Some(header) = header else {
        for entry in real {
            write_output_line(output, entry);
        }
        return;
    };
    if real.len() >= 3 {
        start_output_line(output);
        output.extend_from_slice(header);
        output.extend_from_slice(b"/ (");
        output.extend_from_slice(real.len().to_string().as_bytes());
        output.extend_from_slice(b" entries: ");
        for (position, entry) in real.iter().take(3).enumerate() {
            if position > 0 {
                output.extend_from_slice(b", ");
            }
            output.extend_from_slice(entry);
        }
        write_omission(output, real.len(), 3);
        output.push(b')');
    } else {
        for entry in real {
            start_output_line(output);
            output.extend_from_slice(header);
            output.push(b'/');
            output.extend_from_slice(entry);
        }
    }
}

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

fn apply_tree_compact(input: &[u8]) -> Option<Vec<u8>> {
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

fn write_output_line(output: &mut Vec<u8>, line: &[u8]) {
    start_output_line(output);
    output.extend_from_slice(line);
}

fn start_output_line(output: &mut Vec<u8>) {
    if !output.is_empty() {
        output.push(b'\n');
    }
}

fn write_indent(output: &mut Vec<u8>, depth: usize) {
    for _ in 0..depth {
        output.extend_from_slice(b"  ");
    }
}

fn rg_pattern_separator(line: &[u8]) -> Option<usize> {
    if line.is_empty() || matches!(line[0], b'{' | b' ' | b'\t') {
        return None;
    }
    let mut index = 1;
    while index < line.len() {
        if line[index] != b':' {
            index += 1;
            continue;
        }
        let mut digit = index + 1;
        if !line.get(digit).is_some_and(u8::is_ascii_digit) {
            index += 1;
            continue;
        }
        while line.get(digit).is_some_and(u8::is_ascii_digit) {
            digit += 1;
        }
        if line.get(digit) == Some(&b':') {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn rg_is_file_mode(argv: &[&[u8]]) -> bool {
    argv.iter()
        .any(|argument| matches!(*argument, b"--files" | b"-l" | b"--files-with-matches"))
}

fn rg_requests_exact(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        matches!(
            *argument,
            b"--json"
                | b"--vimgrep"
                | b"-c"
                | b"--count"
                | b"--count-matches"
                | b"--files-without-match"
                | b"--type-list"
                | b"-0"
                | b"--null"
                | b"--null-data"
                | b"-o"
                | b"--only-matching"
                | b"--passthru"
                | b"--stats"
        ) || argument.starts_with(b"--json=")
            || argument.starts_with(b"--replace=")
            || (argument.starts_with(b"-r") && argument.len() > 2)
            || *argument == b"-r"
            || *argument == b"--replace"
    })
}

fn matches_rg_files(input: &[u8]) -> bool {
    if input.is_empty() {
        return false;
    }
    let first = input
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    if first.is_empty() || matches!(first[0], b' ' | b'\t' | 0..=0x1f) {
        return false;
    }
    !first.iter().enumerate().any(|(index, byte)| {
        *byte == b':' && index > 0 && first.get(index + 1).is_some_and(u8::is_ascii_digit)
    })
}

fn apply_rg_files(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous_line: &[u8] = b"";
    let mut previous_dir_len = 0;
    let mut index = 0;
    while index < input.len() {
        let end = input[index..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(input.len(), |relative| index + relative);
        let line = &input[index..end];
        if line.starts_with(b":") {
            output.push(b':');
            output.extend_from_slice(line);
        } else if previous_dir_len > 0
            && line.len() > previous_dir_len
            && line.starts_with(&previous_line[..previous_dir_len])
            && line[previous_dir_len] != b':'
        {
            output.push(b':');
            output.extend_from_slice(&line[previous_dir_len..]);
        } else {
            output.extend_from_slice(line);
        }
        previous_line = line;
        previous_dir_len = line
            .iter()
            .rposition(|byte| *byte == b'/')
            .map_or(0, |separator| separator + 1);
        if end < input.len() {
            output.push(b'\n');
            index = end + 1;
        } else {
            break;
        }
    }
    output
}

fn matches_rg_pattern(input: &[u8]) -> bool {
    input
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
        .and_then(rg_pattern_separator)
        .is_some()
}

fn apply_rg_pattern(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous_path: Option<&[u8]> = None;
    let mut index = 0;
    while index < input.len() {
        let end = input[index..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(input.len(), |relative| index + relative);
        let line = &input[index..end];
        if let Some(separator) = rg_pattern_separator(line) {
            let path = &line[..separator];
            if previous_path == Some(path) {
                output.extend_from_slice(&line[separator..]);
            } else {
                output.extend_from_slice(line);
                previous_path = Some(path);
            }
        } else {
            output.extend_from_slice(line);
            previous_path = None;
        }
        if end < input.len() {
            output.push(b'\n');
            index = end + 1;
        } else {
            break;
        }
    }
    output
}

fn matches_tree(input: &[u8]) -> bool {
    if input.is_empty() || matches!(input[0], b' ' | b'\t' | 0..=0x1f) {
        return false;
    }
    input
        .split(|byte| *byte == b'\n')
        .take(6)
        .any(|line| unicode_tree_line(line) || parse_ascii_tree_line(line).is_some())
}

fn matches_ls_long(input: &[u8]) -> bool {
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

fn apply_ls_long(input: &[u8]) -> Option<Vec<u8>> {
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

fn trim_ascii_end_space(mut input: &[u8]) -> &[u8] {
    while input
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[..input.len() - 1];
    }
    input
}

fn matches_find_ls(input: &[u8]) -> bool {
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

fn apply_find_ls(input: &[u8]) -> Vec<u8> {
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

fn parent_dir(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or(b".", |index| if index == 0 { b"/" } else { &path[..index] })
}

fn basename(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map_or(path, |index| {
            let tail = &path[index + 1..];
            if tail.is_empty() { path } else { tail }
        })
}

fn write_omission(output: &mut Vec<u8>, total: usize, shown: usize) {
    if total <= shown {
        return;
    }
    output.extend_from_slice(b"; ");
    output.extend_from_slice((total - shown).to_string().as_bytes());
    output.extend_from_slice(b" omitted; --raw for all");
}

#[derive(Clone, Copy)]
struct DuRow<'a> {
    number: &'a [u8],
    unit: Option<u8>,
    bytes: u64,
    path: &'a [u8],
}

fn matches_du(input: &[u8]) -> bool {
    let mut saw_any = false;
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if parse_du_line(line).is_none() {
            return false;
        }
        saw_any = true;
    }
    saw_any
}

fn parse_du_line(line: &[u8]) -> Option<DuRow<'_>> {
    let mut index = line.iter().position(|byte| !matches!(byte, b' ' | b'\t'))?;
    let number_start = index;
    let mut saw_dot = false;
    while index < line.len() {
        match line[index] {
            b'0'..=b'9' => index += 1,
            b'.' if !saw_dot => {
                saw_dot = true;
                index += 1;
            }
            _ => break,
        }
    }
    if index == number_start {
        return None;
    }
    let number = &line[number_start..index];
    let mut unit = None;
    if let Some(byte) = line.get(index).copied() {
        let upper = byte.to_ascii_uppercase();
        if matches!(upper, b'K' | b'M' | b'G' | b'T' | b'P' | b'E') {
            unit = Some(upper);
            index += 1;
            if line
                .get(index)
                .is_some_and(|byte| byte.eq_ignore_ascii_case(&b'B'))
            {
                index += 1;
            }
        }
    }
    if line.get(index) != Some(&b'\t') {
        return None;
    }
    let path = trim_ascii_end_space(&line[index + 1..]);
    if path.is_empty() {
        return None;
    }
    let bytes = du_bytes(number, unit)?;
    Some(DuRow {
        number,
        unit,
        bytes,
        path,
    })
}

fn du_bytes(number: &[u8], unit: Option<u8>) -> Option<u64> {
    let multiplier = match unit {
        None | Some(b'K') => 1024_u64,
        Some(b'M') => 1024_u64.pow(2),
        Some(b'G') => 1024_u64.pow(3),
        Some(b'T') => 1024_u64.pow(4),
        Some(b'P') => 1024_u64.pow(5),
        Some(b'E') => 1024_u64.pow(6),
        _ => return None,
    };
    let dot = number.iter().position(|byte| *byte == b'.');
    let integer = parse_u64(dot.map_or(number, |position| &number[..position]))?;
    let fraction = dot
        .and_then(|position| number.get(position + 1).copied())
        .filter(u8::is_ascii_digit)
        .map_or(0, |byte| u64::from(byte - b'0'));
    let tenths = integer.saturating_mul(10).saturating_add(fraction);
    Some(
        tenths
            .saturating_mul(multiplier / 10)
            .saturating_add(tenths.saturating_mul(multiplier % 10) / 10),
    )
}

fn parse_u64(input: &[u8]) -> Option<u64> {
    if input.is_empty() {
        return None;
    }
    input.iter().try_fold(0_u64, |value, byte| {
        byte.is_ascii_digit().then(|| {
            value
                .saturating_mul(10)
                .saturating_add(u64::from(*byte - b'0'))
        })
    })
}

fn apply_du(input: &[u8], sort_descending: bool) -> Vec<u8> {
    let mut rows: Vec<_> = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(parse_du_line)
        .collect();
    if !sort_descending {
        let mut output = Vec::with_capacity(input.len());
        for row in rows {
            write_du_row(&mut output, row, 0);
        }
        return output;
    }

    rows.sort_by_key(|row| std::cmp::Reverse(row.bytes));
    let emitted = rows.len().min(10);
    let prefix_len = common_path_prefix(&rows[..emitted]);
    let mut output = Vec::with_capacity(input.len());
    if prefix_len > 2 {
        output.extend_from_slice(&rows[0].path[..prefix_len]);
        output.push(b'\n');
    }
    for row in &rows[..emitted] {
        write_du_row(
            &mut output,
            *row,
            if prefix_len > 2 { prefix_len } else { 0 },
        );
    }
    if rows.len() > 10 {
        let remaining = rows[10..]
            .iter()
            .fold(0_u64, |total, row| total.saturating_add(row.bytes));
        write_human_size(&mut output, remaining);
        output.extend_from_slice(b"\t(+");
        output.extend_from_slice((rows.len() - 10).to_string().as_bytes());
        output.extend_from_slice(b")\n");
    }
    output
}

fn common_path_prefix(rows: &[DuRow<'_>]) -> usize {
    if rows.len() <= 1 {
        return 0;
    }
    let mut prefix_len = rows[0].path.len();
    for row in &rows[1..] {
        prefix_len = rows[0].path[..prefix_len]
            .iter()
            .zip(row.path)
            .take_while(|(left, right)| left == right)
            .count();
    }
    while prefix_len > 0 && rows[0].path[prefix_len - 1] != b'/' {
        prefix_len -= 1;
    }
    prefix_len
}

fn write_du_row(output: &mut Vec<u8>, row: DuRow<'_>, strip_prefix: usize) {
    write_rounded_number(output, row.number);
    if let Some(unit) = row.unit {
        output.push(unit);
    }
    output.push(b'\t');
    output.extend_from_slice(&row.path[strip_prefix..]);
    output.push(b'\n');
}

fn write_rounded_number(output: &mut Vec<u8>, number: &[u8]) {
    if number.contains(&b'.') || number.len() <= 2 {
        output.extend_from_slice(number);
        return;
    }
    output.extend_from_slice(&number[..2]);
    output.resize(output.len() + number.len() - 2, b'0');
}

fn write_human_size(output: &mut Vec<u8>, bytes: u64) {
    let (unit, suffix) = if bytes >= 1024_u64.pow(4) {
        (1024_u64.pow(4), b'T')
    } else if bytes >= 1024_u64.pow(3) {
        (1024_u64.pow(3), b'G')
    } else if bytes >= 1024_u64.pow(2) {
        (1024_u64.pow(2), b'M')
    } else if bytes >= 1024 {
        (1024, b'K')
    } else {
        output.extend_from_slice(bytes.to_string().as_bytes());
        return;
    };
    let tenths = bytes / (unit / 10);
    output.extend_from_slice((tenths / 10).to_string().as_bytes());
    output.push(b'.');
    output.push(b'0' + (tenths % 10) as u8);
    output.push(suffix);
}

fn unicode_tree_line(line: &[u8]) -> bool {
    line.len() >= 3 && line[0] == 0xe2 && line[1] == 0x94 && matches!(line[2], 0x9c | 0x94 | 0x82)
}

fn parse_ascii_tree_line(line: &[u8]) -> Option<(usize, &[u8])> {
    let mut index = 0;
    let mut depth = 0;
    while line.get(index..index + 4) == Some(b"|   ") || line.get(index..index + 4) == Some(b"    ")
    {
        depth += 1;
        index += 4;
    }
    if !matches!(line.get(index..index + 4), Some(b"|-- ") | Some(b"`-- ")) {
        return None;
    }
    let name = line[index + 4..].trim_ascii();
    (!name.is_empty()).then_some((depth + 1, name))
}

fn tree_prefix_len(line: &[u8]) -> usize {
    let mut index = 0;
    while index < line.len() {
        match line[index] {
            b' ' => index += 1,
            0xc2 if line.get(index + 1) == Some(&0xa0) => index += 2,
            0xe2 if line.get(index + 1) == Some(&0x94) && index + 2 < line.len() => {
                index += 3;
            }
            _ => break,
        }
    }
    index
}

fn tree_depth(prefix: &[u8]) -> usize {
    let mut columns = 0;
    let mut index = 0;
    while index < prefix.len() {
        if prefix[index] == 0xe2 && prefix.get(index + 1) == Some(&0x94) && index + 2 < prefix.len()
        {
            index += 3;
        } else if prefix[index] == 0xc2 && prefix.get(index + 1) == Some(&0xa0) {
            index += 2;
        } else {
            index += 1;
        }
        columns += 1;
    }
    columns / 4
}

fn apply_tree_pipe(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous_depth = 0;
    let mut index = 0;
    while index < input.len() {
        let end = input[index..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(input.len(), |relative| index + relative);
        let line = &input[index..end];
        let prefix_len = tree_prefix_len(line);
        let name = &line[prefix_len..];
        let depth = tree_depth(&line[..prefix_len]);
        if !name.is_empty() {
            if depth == previous_depth && depth > 0 {
                output.push(b'~');
                output.extend_from_slice(name);
            } else {
                for _ in 0..depth {
                    output.extend_from_slice(b"  ");
                }
                output.extend_from_slice(name);
                previous_depth = depth;
            }
        } else if prefix_len == 0 && !line.is_empty() {
            output.extend_from_slice(line);
            previous_depth = 0;
        }
        if end < input.len() {
            output.push(b'\n');
            index = end + 1;
        } else {
            break;
        }
    }
    output
}
