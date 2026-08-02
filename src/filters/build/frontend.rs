use super::*;

#[derive(Debug)]
struct AssetEntry {
    bytes: usize,
    line: Vec<u8>,
}

#[derive(Default)]
struct AssetSummary {
    count: usize,
    top: Vec<AssetEntry>,
}

impl AssetSummary {
    fn add(&mut self, line: &[u8], bytes: usize) {
        self.count += 1;
        let without_gzip = find_subslice(line, b"\xe2\x94\x82 gzip:")
            .map_or(line, |index| trim_ascii_end(&line[..index]));
        let line = compact_spaces(strip_build_prefix(without_gzip));
        let index = self
            .top
            .iter()
            .position(|entry| bytes > entry.bytes)
            .unwrap_or(self.top.len());
        self.top.insert(index, AssetEntry { bytes, line });
        self.top.truncate(5);
    }

    fn write(&self, output: &mut Vec<u8>) {
        if self.count == 0 {
            return;
        }
        output.extend_from_slice(b"assets x");
        output.extend_from_slice(self.count.to_string().as_bytes());
        output.extend_from_slice(b"; largest:\n");
        for entry in &self.top {
            output.extend_from_slice(b"- ");
            append_line(output, &entry.line);
        }
    }
}

pub(super) fn compact_build_output(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if stdout.is_empty() && stderr.is_empty() {
        return Vec::new();
    }
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut assets = AssetSummary::default();
    let mut kept_lines = 0usize;
    for input in [stdout, stderr] {
        scan_build_output(input, &mut output, &mut assets, &mut kept_lines);
    }
    assets.write(&mut output);
    if kept_lines == 0 && assets.count == 0 {
        return b"build complete\n".to_vec();
    }
    head_tail(output, 120, 80)
}

fn scan_build_output(
    input: &[u8],
    output: &mut Vec<u8>,
    assets: &mut AssetSummary,
    kept_lines: &mut usize,
) {
    if input.is_empty() {
        return;
    }
    let mut previous_blank = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let trimmed = trim_ascii_end(&clean);
        if trimmed.is_empty() {
            if !previous_blank && *kept_lines > 0 {
                output.push(b'\n');
                previous_blank = true;
            }
            continue;
        }
        let body = trim_ascii_start(trimmed);
        if should_drop_build_output(body) || is_webpack_built_module_line(body) {
            continue;
        }
        if let Some(bytes) = asset_size_bytes(body) {
            assets.add(body, bytes);
            continue;
        }
        append_line(output, trimmed);
        *kept_lines += 1;
        previous_blank = false;
    }
}

fn should_drop_build_output(line: &[u8]) -> bool {
    if line.ends_with(b" packages in scope") || is_script_header(line) {
        return true;
    }
    [
        b"cache hit, replaying logs ".as_slice(),
        b"cache miss, executing ",
        b"(node:",
        b"(Use ",
        b"transforming...",
        b"rendering chunks (",
        b"computing gzip size (",
        b"Creating an optimized production build",
        b"info  - Linting",
        b"\xe2\x84\xb9 vite v",
        b"\xe2\x84\xb9 rendering chunks",
        b"\xe2\x84\xb9 computing gzip size",
        b"Tasks:",
        b"Duration:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
        || find_subslice(line, b"DeprecationWarning").is_some()
}

fn is_script_header(line: &[u8]) -> bool {
    let Some(body) = line.strip_prefix(b"> ") else {
        return false;
    };
    find_subslice(body, b"@").is_some() && body.contains(&b' ')
        || [
            b"vite ".as_slice(),
            b"next ",
            b"nuxt ",
            b"webpack ",
            b"npm ",
            b"pnpm ",
            b"yarn ",
            b"bun ",
        ]
        .iter()
        .any(|prefix| body.starts_with(prefix))
}

fn asset_size_bytes(line: &[u8]) -> Option<usize> {
    vite_asset_size_bytes(line).or_else(|| webpack_asset_size_bytes(line))
}

fn vite_asset_size_bytes(line: &[u8]) -> Option<usize> {
    let marker = find_subslice(line, b"\xe2\x94\x82 gzip:")?;
    let before = strip_build_prefix(trim_ascii(&line[..marker]));
    let mut tokens = before
        .split(|byte| matches!(byte, b' ' | b'\t'))
        .filter(|token| !token.is_empty());
    let mut previous = None;
    let mut current = None;
    for token in &mut tokens {
        previous = current;
        current = Some(token);
    }
    parse_size_bytes(previous?, current?)
}

fn webpack_asset_size_bytes(line: &[u8]) -> Option<usize> {
    let mut tokens = line
        .split(|byte| matches!(byte, b' ' | b'\t'))
        .filter(|token| !token.is_empty());
    if tokens.next()? != b"asset" {
        return None;
    }
    tokens.next()?;
    parse_size_bytes(tokens.next()?, tokens.next()?)
}

fn is_webpack_built_module_line(line: &[u8]) -> bool {
    find_subslice(line, b"[built]").is_some() && find_subslice(line, b"[code generated]").is_some()
}

fn parse_size_bytes(number: &[u8], unit: &[u8]) -> Option<usize> {
    let multiplier = if unit.eq_ignore_ascii_case(b"B")
        || unit.eq_ignore_ascii_case(b"byte")
        || unit.eq_ignore_ascii_case(b"bytes")
    {
        1u64
    } else if unit.eq_ignore_ascii_case(b"kB") || unit.eq_ignore_ascii_case(b"KB") {
        1024
    } else if unit.eq_ignore_ascii_case(b"MB") {
        1024 * 1024
    } else if unit.eq_ignore_ascii_case(b"GB") {
        1024 * 1024 * 1024
    } else {
        return None;
    };
    let mut whole = 0u64;
    let mut fraction = 0u64;
    let mut scale = 1u64;
    let mut saw_digit = false;
    let mut saw_dot = false;
    for byte in number {
        if *byte == b',' {
            continue;
        }
        if *byte == b'.' {
            if saw_dot {
                return None;
            }
            saw_dot = true;
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        saw_digit = true;
        let digit = u64::from(*byte - b'0');
        if saw_dot {
            if scale < 1_000_000 {
                fraction = fraction * 10 + digit;
                scale *= 10;
            }
        } else {
            whole = whole * 10 + digit;
        }
    }
    if !saw_digit {
        return None;
    }
    usize::try_from(whole * multiplier + fraction * multiplier / scale).ok()
}

fn strip_build_prefix(line: &[u8]) -> &[u8] {
    let line = trim_ascii(line);
    line.strip_prefix(b"\xe2\x84\xb9 ").map_or(line, trim_ascii)
}

fn compact_spaces(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut in_space = false;
    for byte in input {
        if matches!(byte, b' ' | b'\t') {
            in_space = true;
        } else {
            if in_space && !output.is_empty() {
                output.push(b' ');
            }
            output.push(*byte);
            in_space = false;
        }
    }
    output
}

fn head_tail(data: Vec<u8>, head_lines: usize, tail_lines: usize) -> Vec<u8> {
    let newline_count = data.iter().filter(|byte| **byte == b'\n').count();
    let line_count = newline_count + usize::from(!data.is_empty() && !data.ends_with(b"\n"));
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

pub(super) fn matches_build_compact(stdout: &[u8], stderr: &[u8]) -> bool {
    [stdout, stderr]
        .into_iter()
        .flat_map(|input| input.split(|byte| *byte == b'\n'))
        .any(|line| {
            line.starts_with(b"Build Summary: ") || classify_build_line(line) != BuildLine::Other
        })
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum BuildLine {
    CargoProgress,
    CargoCheckProgress,
    CargoVerboseInvocation,
    MakeProgress,
    NinjaProgress(usize),
    GoProgress,
    Kept,
    Other,
}

pub(super) fn classify_build_line(line: &[u8]) -> BuildLine {
    if line.is_empty() {
        return BuildLine::Other;
    }
    if line.starts_with(b"   Compiling ") {
        return BuildLine::CargoProgress;
    }
    if line.starts_with(b"    Checking ") {
        return BuildLine::CargoCheckProgress;
    }
    if line.starts_with(b"     Running `rustc ") {
        return BuildLine::CargoVerboseInvocation;
    }
    if line.starts_with(b"go build:") {
        return BuildLine::GoProgress;
    }
    if let Some(completed) = ninja_completed(line) {
        return BuildLine::NinjaProgress(completed);
    }
    if [
        b"gcc ".as_slice(),
        b"g++ ",
        b"cc ",
        b"clang ",
        b"clang++ ",
        b"CC ",
        b"CXX ",
        b"LD ",
        b"LINK ",
        b"AR ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
    {
        return BuildLine::MakeProgress;
    }
    if [b"error:".as_slice(), b"error[", b"ERROR", b"FAIL"]
        .iter()
        .any(|needle| find_subslice(line, needle).is_some())
        || [b"warning:".as_slice(), b"WARN"]
            .iter()
            .any(|needle| find_subslice(line, needle).is_some())
    {
        return BuildLine::Kept;
    }
    BuildLine::Other
}
