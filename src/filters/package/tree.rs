pub(super) fn compact_package_tree(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let pnpm = is_pnpm_list(stdout);
    let mut root: Option<Vec<u8>> = None;
    let mut dependencies: Vec<Vec<u8>> = Vec::new();
    let mut dependency_count = 0usize;
    let mut nested_rows = 0usize;
    let mut in_section = false;

    for raw in stdout.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = trim_end(&clean);
        if line.is_empty() {
            in_section = false;
            continue;
        }

        if pnpm {
            if line.starts_with(b"Legend:") {
                continue;
            }
            if line.ends_with(b"ependencies:") {
                in_section = true;
                continue;
            }
            if contains_tree_marker(line) {
                nested_rows += 1;
                continue;
            }
            if in_section && let Some(package) = flat_dependency(line) {
                dependency_count += 1;
                if dependencies.len() < 12 {
                    dependencies.push(package);
                }
                continue;
            }
            if root.is_none() && !starts_with_tree_prefix(line) {
                root = Some(line.to_vec());
            }
            continue;
        }

        if root.is_none() && !starts_with_tree_prefix(line) {
            root = Some(line.to_vec());
            continue;
        }
        if let Some(package) = direct_package(line) {
            dependency_count += 1;
            if dependencies.len() < 12 {
                dependencies.push(package.to_vec());
            }
        } else if contains_tree_marker(line) {
            nested_rows += 1;
        }
    }

    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    if let Some(root) = root {
        output.extend_from_slice(&root);
        output.push(b'\n');
    }
    if dependency_count > 0 {
        output.extend_from_slice(b"deps +");
        output.extend_from_slice(dependency_count.to_string().as_bytes());
        if !dependencies.is_empty() {
            output.extend_from_slice(b": ");
            for (index, dependency) in dependencies.iter().enumerate() {
                if index > 0 {
                    output.extend_from_slice(b", ");
                }
                output.extend_from_slice(dependency);
            }
            if dependency_count > dependencies.len() {
                output.extend_from_slice(b", ...");
            }
        }
        output.push(b'\n');
    }
    if nested_rows > 0 {
        output.extend_from_slice(b"nested rows x");
        output.extend_from_slice(nested_rows.to_string().as_bytes());
        output.push(b'\n');
    }
    output.extend_from_slice(stderr);
    output
}

pub(super) fn compact_pnpm_list(stdout: &[u8]) -> Option<Vec<u8>> {
    let mut saw_legend = false;
    let mut saw_root = false;
    let mut in_section = false;
    let mut dependency_rows = 0usize;
    for raw in stdout.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = trim_end(&clean).trim_ascii_start();
        if line.is_empty() {
            continue;
        }
        if !saw_legend {
            if !line.starts_with(b"Legend:") {
                return None;
            }
            saw_legend = true;
            continue;
        }
        if !saw_root {
            if !line.contains(&b'@') || line.starts_with(b"Legend:") {
                return None;
            }
            saw_root = true;
            continue;
        }
        if matches!(
            line,
            b"dependencies:" | b"devDependencies:" | b"optionalDependencies:"
        ) {
            in_section = true;
            continue;
        }
        if !in_section {
            return None;
        }
        if contains_tree_marker(line) || flat_dependency(line).is_some() {
            dependency_rows += 1;
        } else {
            return None;
        }
    }
    (saw_legend && saw_root && in_section && dependency_rows > 0)
        .then(|| compact_package_tree(stdout, b""))
}

fn direct_package(line: &[u8]) -> Option<&[u8]> {
    TREE_PREFIXES
        .iter()
        .find_map(|prefix| line.strip_prefix(*prefix).map(trim_ascii))
}

fn flat_dependency(line: &[u8]) -> Option<Vec<u8>> {
    let space = line.iter().position(|byte| *byte == b' ')?;
    let name = &line[..space];
    let version = trim_ascii(&line[space + 1..]);
    if name.is_empty() || version.is_empty() {
        return None;
    }
    let mut package = Vec::with_capacity(name.len() + version.len() + 1);
    package.extend_from_slice(name);
    package.push(b'@');
    package.extend_from_slice(version);
    Some(package)
}

fn starts_with_tree_prefix(line: &[u8]) -> bool {
    line.starts_with(b"\xe2\x94")
}

pub(super) fn contains_tree_marker(line: &[u8]) -> bool {
    TREE_PREFIXES
        .iter()
        .any(|prefix| find_subslice(line, prefix).is_some())
}

pub(super) fn is_pnpm_list(input: &[u8]) -> bool {
    input.starts_with(b"Legend:")
        || find_subslice(input, b"\ndependencies:").is_some()
        || find_subslice(input, b"\ndevDependencies:").is_some()
        || find_subslice(input, b"\noptionalDependencies:").is_some()
}
use super::exact::trim_ascii;
use super::{TREE_PREFIXES, find_subslice, strip_ansi, trim_end};
