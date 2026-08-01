use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterOutput, append_line, byte_after_lines,
    command_basename, find_subslice, strip_ansi, trim_ascii_end_space as trim_end,
};

const TREE_PREFIXES: &[&[u8]] = &[
    b"\xe2\x94\x9c\xe2\x94\x80\xe2\x94\x80 ",
    b"\xe2\x94\x94\xe2\x94\x80\xe2\x94\x80 ",
    b"\xe2\x94\x9c\xe2\x94\x80\xe2\x94\xac ",
    b"\xe2\x94\x94\xe2\x94\x80\xe2\x94\xac ",
    b"\xe2\x94\x9c\xe2\x94\x80 ",
    b"\xe2\x94\x94\xe2\x94\x80 ",
];

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    argv.first()
        .copied()
        .map(command_basename)
        .is_some_and(|command| {
            matches!(
                command,
                b"npm" | b"pnpm" | b"yarn" | b"bun" | b"composer" | b"pip" | b"pip3"
            )
        })
}

pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    if argv.is_empty() {
        return Err(FilterError::InvalidInput);
    }
    if lossless || requests_exact_output(argv) {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }

    let command = command_basename(argv[0]);
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let arg2 = argv.get(2).copied().unwrap_or_default();
    let recognized_error = has_package_error_marker(stdout) || has_package_error_marker(stderr);
    if exit_code != 0 && !stderr.is_empty() && !recognized_error {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }

    let package_tree_route = command == b"bun" && arg1 == b"pm" && arg2 == b"ls"
        || matches!(command, b"npm" | b"pnpm") && matches!(arg1, b"ls" | b"list")
        || command == b"yarn" && arg1 == b"list";
    if package_tree_route && matches_package_tree(stdout) {
        return Ok(StreamFilterOutput::new(
            compact_package_tree(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::PotentiallyLossy,
        ));
    }

    let js_install_route = matches!(command, b"npm" | b"pnpm" | b"yarn" | b"bun")
        && matches!(arg1, b"install" | b"i" | b"ci" | b"add" | b"remove" | b"rm");
    let composer_route = command == b"composer"
        && matches!(
            arg1,
            b"install" | b"require" | b"update" | b"upgrade" | b"remove" | b"create-project"
        );
    if (js_install_route || composer_route)
        && (matches_npm_install(stdout) || matches_npm_install(stderr) || recognized_error)
    {
        let evidence = if exit_code == 0 {
            EvidenceClass::PotentiallyLossy
        } else {
            EvidenceClass::FactComplete
        };
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            evidence,
            compact_npm_install,
        ));
    }

    let pip_command = matches!(command, b"pip" | b"pip3");
    let pip_table_route = pip_command && matches!(arg1, b"list" | b"outdated");
    if pip_table_route {
        return Ok(StreamFilterOutput::new(
            compact_pip_table(stdout, b""),
            stderr.to_vec(),
            EvidenceClass::FactComplete,
        ));
    }
    let pip_install_route = pip_command && matches!(arg1, b"install" | b"download" | b"wheel");
    if pip_install_route
        && (looks_like_pip_install(stdout) || looks_like_pip_install(stderr) || recognized_error)
    {
        let evidence = if exit_code == 0 {
            EvidenceClass::PotentiallyLossy
        } else {
            EvidenceClass::FactComplete
        };
        return Ok(StreamFilterOutput::compact_single_stream(
            stdout,
            stderr,
            evidence,
            compact_pip,
        ));
    }

    Ok(StreamFilterOutput::passthrough(stdout, stderr))
}

pub fn matches_pipe(input: &[u8]) -> bool {
    matches_npm_install(input)
}

pub fn apply_pipe(input: &[u8]) -> Result<FilterOutput, FilterError> {
    if !matches_pipe(input) {
        return Err(FilterError::InvalidInput);
    }
    Ok(FilterOutput::new(
        compact_npm_install(input, b""),
        EvidenceClass::PotentiallyLossy,
    ))
}

fn compact_pip_table(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if stdout.is_empty() {
        return stderr.to_vec();
    }
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for raw in stdout.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let line = trim_ascii(raw);
        if line.is_empty() || is_dash_separator(line) || is_pip_header(line) {
            continue;
        }
        write_collapsed(line, &mut output);
        output.push(b'\n');
    }
    output.extend_from_slice(stderr);
    output
}

fn is_dash_separator(line: &[u8]) -> bool {
    let mut saw_dash = false;
    for byte in line {
        if *byte == b'-' {
            saw_dash = true;
        } else if !matches!(byte, b' ' | b'\t') {
            return false;
        }
    }
    saw_dash
}

fn is_pip_header(line: &[u8]) -> bool {
    starts_with_ignore_ascii_case(line, b"Package ")
        || line.eq_ignore_ascii_case(b"Package Version")
}

fn starts_with_ignore_ascii_case(input: &[u8], prefix: &[u8]) -> bool {
    input
        .get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

fn write_collapsed(line: &[u8], output: &mut Vec<u8>) {
    let mut first = true;
    for token in line.split(|byte| matches!(byte, b' ' | b'\t')) {
        if token.is_empty() {
            continue;
        }
        if !first {
            output.push(b' ');
        }
        first = false;
        output.extend_from_slice(token);
    }
}

fn looks_like_pip_install(input: &[u8]) -> bool {
    [
        b"Collecting ".as_slice(),
        b"Downloading ",
        b"Installing collected packages:",
        b"Successfully installed ",
        b"Requirement already satisfied:",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

#[derive(Default)]
struct PipSamples {
    total: usize,
    items: Vec<Vec<u8>>,
}

impl PipSamples {
    fn add(&mut self, item: &[u8]) {
        if item.is_empty() {
            return;
        }
        self.total += 1;
        if self.items.len() < 8 {
            self.items.push(item.to_vec());
        }
    }

    fn add_counted_sample(&mut self, item: &[u8]) {
        if item.is_empty() {
            return;
        }
        self.total += 1;
        if self.items.len() < 8 && !self.items.iter().any(|existing| existing == item) {
            self.items.push(item.to_vec());
        }
    }
}

fn compact_pip(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if !looks_like_pip_install(stdout) && !looks_like_pip_install(stderr) {
        return compact_pip_table(stdout, stderr);
    }

    let mut collected = PipSamples::default();
    let mut satisfied = PipSamples::default();
    let mut installing = PipSamples::default();
    let mut downloads = 0usize;
    let mut important = Vec::new();
    for input in [stdout, stderr] {
        scan_pip_install(
            input,
            &mut collected,
            &mut satisfied,
            &mut installing,
            &mut downloads,
            &mut important,
        );
    }

    let mut output = Vec::new();
    write_pip_samples(&mut output, b"Collecting", &collected);
    if downloads > 0 {
        output.extend_from_slice(b"Downloaded ");
        output.extend_from_slice(downloads.to_string().as_bytes());
        output.extend_from_slice(b" files\n");
    }
    write_pip_samples(&mut output, b"Satisfied", &satisfied);
    write_pip_samples(&mut output, b"Installing", &installing);
    output.extend_from_slice(&important);
    output
}

fn scan_pip_install(
    input: &[u8],
    collected: &mut PipSamples,
    satisfied: &mut PipSamples,
    installing: &mut PipSamples,
    downloads: &mut usize,
    important: &mut Vec<u8>,
) {
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = trim_ascii(&clean);
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix(b"Collecting ") {
            collected.add(first_token(rest));
        } else if line.starts_with(b"Downloading ") {
            *downloads += 1;
        } else if let Some(rest) = line.strip_prefix(b"Requirement already satisfied: ") {
            satisfied.add_counted_sample(satisfied_name(rest));
        } else if let Some(rest) = line.strip_prefix(b"Installing collected packages:") {
            for raw_name in rest.split(|byte| *byte == b',') {
                installing.add(trim_pip_package_name(raw_name));
            }
        } else if !is_pip_progress_line(line) {
            append_line(important, line);
        }
    }
}

fn satisfied_name(input: &[u8]) -> &[u8] {
    let input = trim_ascii(input);
    if let Some(index) = find_subslice(input, b" in ") {
        return trim_ascii(&input[..index]);
    }
    if let Some(index) = find_subslice(input, b" (from ") {
        return trim_ascii(&input[..index]);
    }
    first_token(input)
}

fn trim_pip_package_name(mut input: &[u8]) -> &[u8] {
    while input
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b','))
    {
        input = &input[1..];
    }
    while input
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b','))
    {
        input = &input[..input.len() - 1];
    }
    input
}

fn is_pip_progress_line(line: &[u8]) -> bool {
    find_subslice(line, b" eta ").is_some()
        || line
            .split(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
            .any(is_speed_token)
}

fn is_speed_token(token: &[u8]) -> bool {
    let token = token.trim_ascii_matches(b".,;()[]");
    [
        b"B/s".as_slice(),
        b"kB/s",
        b"KB/s",
        b"MB/s",
        b"GB/s",
        b"KiB/s",
        b"MiB/s",
        b"GiB/s",
    ]
    .iter()
    .any(|unit| {
        token == *unit
            || token
                .strip_suffix(*unit)
                .is_some_and(|prefix| prefix.iter().any(u8::is_ascii_digit))
    })
}

fn write_pip_samples(output: &mut Vec<u8>, label: &[u8], samples: &PipSamples) {
    if samples.total == 0 {
        return;
    }
    output.extend_from_slice(label);
    output.push(b' ');
    output.extend_from_slice(samples.total.to_string().as_bytes());
    output.extend_from_slice(b": ");
    for (index, item) in samples.items.iter().enumerate() {
        if index > 0 {
            output.extend_from_slice(b", ");
        }
        output.extend_from_slice(item);
    }
    if samples.total > samples.items.len() {
        output.extend_from_slice(b", ... (+");
        output.extend_from_slice((samples.total - samples.items.len()).to_string().as_bytes());
        output.extend_from_slice(b")");
    }
    output.push(b'\n');
}

trait ByteSliceExt {
    fn trim_ascii_matches(&self, matches: &[u8]) -> &[u8];
}

impl ByteSliceExt for [u8] {
    fn trim_ascii_matches(&self, matches: &[u8]) -> &[u8] {
        let start = self
            .iter()
            .position(|byte| !matches.contains(byte))
            .unwrap_or(self.len());
        let end = self
            .iter()
            .rposition(|byte| !matches.contains(byte))
            .map_or(start, |index| index + 1);
        &self[start..end]
    }
}

fn matches_npm_install(input: &[u8]) -> bool {
    find_subslice(input, b"added ").is_some() && find_subslice(input, b"packages").is_some()
        || input.split(|byte| *byte == b'\n').any(|line| {
            find_subslice(line, b"up to date").is_some()
                && (find_subslice(line, b"audited").is_some()
                    || find_subslice(line, b"packages").is_some())
        })
        || find_subslice(input, b"audited ").is_some()
            && find_subslice(input, b"packages").is_some()
        || [
            b"npm error".as_slice(),
            b"npm ERR!",
            b"npm WARN",
            b"Packages: +",
            b"Packages: -",
            b"Already up to date",
            b" packages installed [",
            b"success Saved ",
            b"Package operations:",
            b"Lock file operations:",
            b"Nothing to install",
            b"No security vulnerability",
            b"Your requirements could not be resolved",
        ]
        .iter()
        .any(|needle| find_subslice(input, needle).is_some())
        || find_subslice(input, b"Done in ").is_some() && find_subslice(input, b"s.").is_some()
}

fn compact_npm_install(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if (looks_like_pnpm(stdout) || looks_like_pnpm(stderr))
        && let Some(output) = compact_pnpm(stdout, stderr)
    {
        return output;
    }
    if looks_like_npm(stdout) || looks_like_npm(stderr) {
        return compact_npm(stdout, stderr);
    }
    if (looks_like_bun_yarn(stdout) || looks_like_bun_yarn(stderr))
        && let Some(output) = compact_bun_yarn(stdout, stderr)
    {
        return output;
    }
    let mut output = Vec::new();
    let mut kept_lines = 0usize;
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            let clean = strip_ansi(raw);
            let line = trim_ascii(&clean);
            if should_keep_install_line(line) {
                output.extend_from_slice(line);
                output.push(b'\n');
                kept_lines += 1;
            }
        }
    }
    if output.is_empty() {
        output.extend_from_slice(b"up to date\n");
    }
    head_tail(output, kept_lines, 40, 20)
}

fn looks_like_pnpm(input: &[u8]) -> bool {
    [
        b"Packages: +".as_slice(),
        b"Packages: -",
        b"Progress: ",
        b"Lockfile is up to date",
        b"\ndependencies:\n",
        b"\ndevDependencies:\n",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

fn compact_pnpm(stdout: &[u8], stderr: &[u8]) -> Option<Vec<u8>> {
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

fn looks_like_bun_yarn(input: &[u8]) -> bool {
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

fn compact_bun_yarn(stdout: &[u8], stderr: &[u8]) -> Option<Vec<u8>> {
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

fn first_token(input: &[u8]) -> &[u8] {
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

fn looks_like_npm(input: &[u8]) -> bool {
    [
        b"npm WARN".as_slice(),
        b"npm notice",
        b"audited ",
        b"run `npm audit`",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

fn compact_npm(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

fn deprecated_package_name(rest: &[u8]) -> &[u8] {
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

fn write_name_summary(
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

fn should_keep_install_line(_line: &[u8]) -> bool {
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

fn head_tail(data: Vec<u8>, line_count: usize, head_lines: usize, tail_lines: usize) -> Vec<u8> {
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

fn has_package_error_marker(input: &[u8]) -> bool {
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

fn matches_package_tree(input: &[u8]) -> bool {
    contains_tree_marker(input) || is_pnpm_list(input)
}

fn compact_package_tree(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

fn contains_tree_marker(line: &[u8]) -> bool {
    TREE_PREFIXES
        .iter()
        .any(|prefix| find_subslice(line, prefix).is_some())
}

fn is_pnpm_list(input: &[u8]) -> bool {
    input.starts_with(b"Legend:")
        || find_subslice(input, b"\ndependencies:").is_some()
        || find_subslice(input, b"\ndevDependencies:").is_some()
        || find_subslice(input, b"\noptionalDependencies:").is_some()
}

fn requests_exact_output(argv: &[&[u8]]) -> bool {
    for argument in &argv[1..] {
        if *argument == b"--" {
            break;
        }
        if matches!(*argument, b"--help" | b"--version" | b"-h" | b"-V") {
            return true;
        }
        if matches!(
            *argument,
            b"--json"
                | b"--ndjson"
                | b"--parseable"
                | b"--porcelain"
                | b"--json-stream"
                | b"--format"
                | b"--reporter"
        ) || argument.starts_with(b"--json=")
            || argument.starts_with(b"--format=")
            || argument.starts_with(b"--reporter=")
        {
            return true;
        }
    }
    matches!(argv.get(1), Some(&b"help") | Some(&b"version"))
}

fn trim_ascii(mut input: &[u8]) -> &[u8] {
    while input
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[1..];
    }
    trim_end(input)
}
