use super::*;

pub(super) fn compact_pip_table(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

pub(super) fn looks_like_pip_install(input: &[u8]) -> bool {
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

pub(super) fn compact_pip(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

pub(super) fn matches_npm_install(input: &[u8]) -> bool {
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
