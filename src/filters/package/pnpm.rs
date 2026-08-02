use super::*;

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
