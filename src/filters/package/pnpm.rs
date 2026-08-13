pub(super) fn compact_pnpm(stdout: &[u8], stderr: &[u8]) -> Option<Vec<u8>> {
    #[derive(Clone, Copy)]
    enum Section {
        None,
        Dependencies,
        DevDependencies,
    }

    let mut head = Vec::new();
    let mut deprecations = Vec::new();
    let mut deprecation_count = 0usize;
    let mut dependencies = Vec::new();
    let mut dependency_count = 0usize;
    let mut dev_dependencies = Vec::new();
    let mut dev_dependency_count = 0usize;
    let mut tail = Vec::new();

    for input in [stdout, stderr] {
        let mut section = Section::None;
        for raw in input.split(|byte| *byte == b'\n') {
            let clean = strip_ansi(raw);
            let line = trim_ascii(&clean);
            if line.is_empty() {
                continue;
            }
            if line == b"dependencies:" {
                section = Section::Dependencies;
                continue;
            }
            if line == b"devDependencies:" {
                section = Section::DevDependencies;
                continue;
            }
            if let Some(rest) = line.strip_prefix(b"+ ") {
                let entry = pnpm_dependency_entry(rest).to_vec();
                match section {
                    Section::Dependencies => {
                        dependency_count += 1;
                        if dependencies.len() < 8 {
                            dependencies.push(entry);
                        }
                    }
                    Section::DevDependencies => {
                        dev_dependency_count += 1;
                        if dev_dependencies.len() < 8 {
                            dev_dependencies.push(entry);
                        }
                    }
                    Section::None => {}
                }
                continue;
            }
            section = Section::None;

            if line.starts_with(b"The following dependencies have build scripts that were ignored")
            {
                append_line(&mut head, line);
            } else if line.starts_with(b"WARN") {
                if let Some(index) = find_subslice(line, b"deprecated ") {
                    deprecation_count += 1;
                    if deprecations.len() < 8 {
                        deprecations.push(
                            deprecated_package_name(&line[index + b"deprecated ".len()..]).to_vec(),
                        );
                    }
                } else {
                    append_line(&mut head, line);
                }
            } else if line.starts_with(b"ERROR ") {
                append_line(&mut head, line);
            } else if line.starts_with(b"added ")
                || line.starts_with(b"removed ")
                || line.starts_with(b"changed ")
                || line.starts_with(b"audited ")
                || line.starts_with(b"found ")
            {
                append_line(&mut tail, line);
            }
        }
    }

    let mut output = head;
    write_name_summary(
        &mut output,
        b"deprecated",
        b'x',
        deprecation_count,
        &deprecations,
    );
    write_name_summary(&mut output, b"deps", b'+', dependency_count, &dependencies);
    write_name_summary(
        &mut output,
        b"dev",
        b'+',
        dev_dependency_count,
        &dev_dependencies,
    );
    output.extend_from_slice(&tail);
    (!output.is_empty()).then_some(output)
}

fn pnpm_dependency_entry(rest: &[u8]) -> &[u8] {
    let mut index = 0usize;
    while index < rest.len() && !matches!(rest[index], b' ' | b'\t') {
        index += 1;
    }
    while index < rest.len() && matches!(rest[index], b' ' | b'\t') {
        index += 1;
    }
    while index < rest.len() && !matches!(rest[index], b' ' | b'\t') {
        index += 1;
    }
    trim_end(&rest[..index])
}

pub(super) fn looks_like_bun_yarn(input: &[u8]) -> bool {
    [
        b"bun add v".as_slice(),
        b"bun install v",
        b" packages installed [",
        b"yarn add v",
        b"success Saved ",
        b"info Direct dependencies",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}
use super::exact::trim_ascii;
use super::npm::{deprecated_package_name, write_name_summary};
use super::{append_line, find_subslice, strip_ansi, trim_end};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Route {
    List,
    Outdated,
    Exact,
    Other,
}

impl Route {
    pub(super) fn is_human(self) -> bool {
        matches!(self, Self::List | Self::Outdated)
    }
}

pub(super) fn classify(argv: &[&[u8]]) -> Route {
    let subcommand = argv.get(1).copied().unwrap_or_default();
    if !matches!(subcommand, b"list" | b"ls" | b"outdated") {
        return Route::Other;
    }
    let arguments = crate::invocation_policy::options(argv);
    if arguments.iter().any(|argument| {
        matches!(
            *argument,
            b"--json"
                | b"--parseable"
                | b"--ndjson"
                | b"--porcelain"
                | b"--json-stream"
                | b"--reporter"
        ) || argument.starts_with(b"--json=")
            || argument.starts_with(b"--parseable=")
            || argument.starts_with(b"--reporter=")
    }) {
        return Route::Exact;
    }

    let mut format = None;
    let mut index = 0usize;
    while index < arguments.len() {
        let argument = arguments[index];
        if argument == b"--format" {
            let Some(value) = arguments.get(index + 1).copied() else {
                return Route::Exact;
            };
            if format.replace(value).is_some() {
                return Route::Exact;
            }
            index += 2;
            continue;
        }
        if let Some(value) = argument.strip_prefix(b"--format=")
            && (value.is_empty() || format.replace(value).is_some())
        {
            return Route::Exact;
        }
        index += 1;
    }
    if matches!(subcommand, b"list" | b"ls") && format.is_some()
        || format.is_some_and(|format| format != b"table")
    {
        return Route::Exact;
    }

    if matches!(subcommand, b"list" | b"ls") {
        Route::List
    } else {
        Route::Outdated
    }
}

pub(super) fn compact_outdated(input: &[u8]) -> Option<Vec<u8>> {
    let input = std::str::from_utf8(input).ok()?;
    let lines = input
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 5 || !box_border(lines[0], '┌', '┐') || !box_border(lines.last()?, '└', '┘')
    {
        return None;
    }
    let header = box_cells(lines[1])?;
    if header != ["Package", "Current", "Latest"] || !box_border(lines[2], '├', '┤') {
        return None;
    }

    let mut output = b"Package Current Latest\n".to_vec();
    let rows = &lines[3..lines.len() - 1];
    if rows.is_empty() {
        return None;
    }
    for line in rows {
        let cells = box_cells(line)?;
        if cells.len() != 3 || cells.iter().any(|cell| cell.is_empty()) {
            return None;
        }
        output.extend_from_slice(cells.join(" ").as_bytes());
        output.push(b'\n');
    }
    Some(output)
}

fn box_cells(line: &str) -> Option<Vec<&str>> {
    if !line.starts_with('│') || !line.ends_with('│') {
        return None;
    }
    Some(line.trim_matches('│').split('│').map(str::trim).collect())
}

fn box_border(line: &str, start: char, end: char) -> bool {
    line.starts_with(start)
        && line.ends_with(end)
        && line
            .chars()
            .skip(1)
            .take(line.chars().count().saturating_sub(2))
            .all(|character| matches!(character, '─' | '┬' | '┼' | '┴'))
}
