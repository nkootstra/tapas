pub(super) fn compact_npm(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut deprecations = Vec::new();
    let mut deprecation_count = 0usize;
    let mut lines = Vec::new();
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            let clean = strip_ansi(raw);
            let line = trim_ascii(&clean);
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix(b"npm WARN deprecated ") {
                deprecation_count += 1;
                if deprecations.len() < 8 {
                    deprecations.push(deprecated_package_name(rest).to_vec());
                }
                continue;
            }
            if line.starts_with(b"npm WARN")
                || line.starts_with(b"npm ERR!")
                || line.starts_with(b"npm error")
                || line.starts_with(b"npm err!")
                || line.starts_with(b"added ")
                || line.starts_with(b"removed ")
                || line.starts_with(b"changed ")
                || line.starts_with(b"up to date")
                || line.starts_with(b"up-to-date")
                || line.starts_with(b"audited ")
                || line.starts_with(b"found ")
            {
                lines.extend_from_slice(line);
                lines.push(b'\n');
            }
        }
    }

    let mut output = Vec::new();
    write_name_summary(
        &mut output,
        b"deprecated",
        b'x',
        deprecation_count,
        &deprecations,
    );
    output.extend_from_slice(&lines);
    output
}

pub(super) fn deprecated_package_name(rest: &[u8]) -> &[u8] {
    let token_end = rest
        .iter()
        .position(|byte| matches!(byte, b':' | b' ' | b'\t'))
        .unwrap_or(rest.len());
    let token = &rest[..token_end];
    token
        .iter()
        .rposition(|byte| *byte == b'@')
        .filter(|index| *index > 0)
        .map_or(token, |at| &token[..at])
}

pub(super) fn write_name_summary(
    output: &mut Vec<u8>,
    label: &[u8],
    sigil: u8,
    count: usize,
    items: &[Vec<u8>],
) {
    if count == 0 {
        return;
    }
    output.extend_from_slice(label);
    output.push(b' ');
    output.push(sigil);
    output.extend_from_slice(count.to_string().as_bytes());
    if !items.is_empty() {
        output.extend_from_slice(b": ");
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                output.extend_from_slice(b", ");
            }
            output.extend_from_slice(item);
        }
        if count > items.len() {
            output.extend_from_slice(b", ...");
        }
    }
    output.push(b'\n');
}

pub(super) fn should_keep_install_line(_line: &[u8]) -> bool {
    let line = _line;
    if line.starts_with(b"npm notice")
        || find_subslice(line, b"packages are looking for funding").is_some()
        || line.starts_with(b"run `npm ")
        || line.starts_with(b"Progress: ")
        || line.starts_with(b"Lockfile is up to date")
        || line.starts_with(b"bun add v")
        || line.starts_with(b"bun install v")
        || line.starts_with(b"bun remove v")
        || line.starts_with(b"yarn add v")
        || line.starts_with(b"yarn install v")
        || line.starts_with(b"yarn remove v")
        || line.starts_with(b"[1/4]")
        || line.starts_with(b"[2/4]")
        || line.starts_with(b"[3/4]")
        || line.starts_with(b"[4/4]")
        || line.starts_with(b"info ")
        || line.starts_with(b"installed ")
        || line.starts_with(b"Loading composer repositories")
        || line.starts_with(b"Updating dependencies")
        || line.starts_with(b"Installing dependencies from lock file")
        || line.starts_with(b"Writing lock file")
        || line.starts_with(b"Generating ")
        || line.starts_with(b"Verifying lock file")
        || line.starts_with(b"Running composer ")
        || line.starts_with(b"Discovered Package:")
        || line.starts_with(b"Use the `composer ")
        || line.starts_with(b"./composer.json has been updated")
        || line.starts_with(b"> @")
        || line.starts_with(b"- Downloading ")
        || line.starts_with(b"- Installing ")
        || line.starts_with(b"- Locking ")
        || line.starts_with(b"- Removing ")
        || find_subslice(line, b"packages you rely on are looking for funding").is_some()
    {
        return false;
    }
    if line.starts_with(b"Using version ")
        || find_subslice(line, b" packages installed [").is_some()
    {
        return true;
    }
    [
        b"npm WARN".as_slice(),
        b"npm ERR!",
        b"npm error",
        b"npm err!",
        b"WARN ",
        b"ERROR ",
        b"warn:",
        b"error:",
        b"warning ",
        b"error ",
        b"added ",
        b"removed ",
        b"changed ",
        b"up to date",
        b"up-to-date",
        b"Already up to date",
        b"audited ",
        b"found 0 vulnerabilities",
        b"found ",
        b"Packages: ",
        b"Done in ",
        b"success ",
        b"Package operations:",
        b"Lock file operations:",
        b"Nothing to install",
        b"No security vulnerability",
        b"Your requirements could not be resolved",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

pub(super) fn head_tail(
    data: Vec<u8>,
    line_count: usize,
    head_lines: usize,
    tail_lines: usize,
) -> Vec<u8> {
    if line_count <= head_lines + tail_lines {
        return data;
    }
    let omitted = line_count - head_lines - tail_lines;
    let head_end = byte_after_lines(&data, head_lines);
    let tail_start = byte_after_lines(&data, line_count - tail_lines);
    let mut output = Vec::with_capacity(data.len());
    output.extend_from_slice(&data[..head_end]);
    output.extend_from_slice(b"(tapas: omitted ");
    output.extend_from_slice(omitted.to_string().as_bytes());
    output.extend_from_slice(b" relevant lines; rerun with tapas --raw)\n");
    output.extend_from_slice(&data[tail_start..]);
    output
}

pub(super) fn has_package_error_marker(input: &[u8]) -> bool {
    [
        b"npm ERR!".as_slice(),
        b"npm error",
        b"npm err!",
        b"ERROR ",
        b"error:",
        b"error ",
        b"Your requirements could not be resolved",
        b"ERROR:",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

pub(super) fn matches_package_tree(input: &[u8]) -> bool {
    contains_tree_marker(input) || is_pnpm_list(input)
}
use super::exact::trim_ascii;
use super::tree::{contains_tree_marker, is_pnpm_list};
use super::{byte_after_lines, find_subslice, strip_ansi};
