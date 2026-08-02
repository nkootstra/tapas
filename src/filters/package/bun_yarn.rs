pub(super) fn compact_bun_yarn(stdout: &[u8], stderr: &[u8]) -> Option<Vec<u8>> {
    let mut head = Vec::new();
    let mut dependencies = Vec::new();
    let mut dependency_count = 0usize;
    let mut tail = Vec::new();

    for input in [stdout, stderr] {
        let mut in_yarn_direct_dependencies = false;
        for raw in input.split(|byte| *byte == b'\n') {
            let clean = strip_ansi(raw);
            let line = trim_ascii(&clean);
            if line.is_empty() {
                continue;
            }
            if line.starts_with(b"warn:")
                || line.starts_with(b"error:")
                || line.starts_with(b"warning ")
                || line.starts_with(b"error ")
            {
                append_line(&mut head, line);
                continue;
            }
            if let Some(rest) = line.strip_prefix(b"installed ") {
                let package = first_token(rest);
                if !package.is_empty() {
                    dependency_count += 1;
                    if dependencies.len() < 8 {
                        dependencies.push(package.to_vec());
                    }
                }
                continue;
            }
            if line == b"info Direct dependencies" {
                in_yarn_direct_dependencies = true;
                continue;
            }
            if line == b"info All dependencies" {
                in_yarn_direct_dependencies = false;
                continue;
            }
            if in_yarn_direct_dependencies {
                if let Some(package) = yarn_tree_package(line) {
                    dependency_count += 1;
                    if dependencies.len() < 8 {
                        dependencies.push(package.to_vec());
                    }
                }
                continue;
            }
            if line.starts_with(b"success Saved ") && find_subslice(line, b"lockfile").is_none()
                || line.starts_with(b"Done in ")
                || find_subslice(line, b" packages installed [").is_some()
            {
                append_line(&mut tail, line);
            }
        }
    }

    let mut output = head;
    write_name_summary(&mut output, b"deps", b'+', dependency_count, &dependencies);
    output.extend_from_slice(&tail);
    (!output.is_empty()).then_some(output)
}

pub(super) fn first_token(input: &[u8]) -> &[u8] {
    let input = trim_ascii(input);
    let end = input
        .iter()
        .position(|byte| matches!(byte, b' ' | b'\t'))
        .unwrap_or(input.len());
    &input[..end]
}

fn yarn_tree_package(line: &[u8]) -> Option<&[u8]> {
    let start = line
        .iter()
        .position(|byte| byte.is_ascii_alphanumeric() || *byte == b'@')?;
    let rest = &line[start..];
    let end = rest
        .iter()
        .position(|byte| matches!(byte, b' ' | b'\t'))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

pub(super) fn looks_like_npm(input: &[u8]) -> bool {
    [
        b"npm WARN".as_slice(),
        b"npm notice",
        b"audited ",
        b"run `npm audit`",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}
use super::exact::trim_ascii;
use super::npm::write_name_summary;
use super::{append_line, find_subslice, strip_ansi};
