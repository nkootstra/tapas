use super::{EvidenceClass, FilterError, StreamFilterOutput};

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
        return Ok(passthrough(stdout, stderr));
    }

    let command = command_basename(argv[0]);
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let runner_package_prelude = matches!(command, b"uv" | b"uvx" | b"poetry" | b"pnpm" | b"npx")
        && (has_package_prelude(stdout) || has_package_prelude(stderr));
    let recognized_failure = has_recognized_failure(stdout) || has_recognized_failure(stderr);
    if exit_code != 0 && !stderr.is_empty() && !recognized_failure {
        return Ok(passthrough(stdout, stderr));
    }
    let generic_build_route = matches!(command, b"make" | b"ninja")
        || command == b"cargo" && matches!(arg1, b"build" | b"check" | b"clippy")
        || command == b"go" && arg1 == b"build"
        || command == b"zig" && arg1 == b"build";
    if generic_build_route && matches_build_compact(stdout, stderr) {
        return Ok(StreamFilterOutput::new(
            compact_build(stdout, stderr),
            Vec::new(),
            compact_evidence(exit_code),
        ));
    }

    let js_build_route = matches!(command, b"npm" | b"pnpm" | b"yarn" | b"bun")
        && (arg1 == b"build"
            || arg1 == b"run" && argv.get(2).copied().unwrap_or_default() == b"build");
    let frontend_build_route = command == b"webpack"
        || command == b"turbo"
        || command == b"next" && arg1 == b"build"
        || js_build_route;
    if frontend_build_route && (matches_build_output(stdout) || matches_build_output(stderr)) {
        return Ok(StreamFilterOutput::new(
            compact_build_output(stdout, stderr),
            Vec::new(),
            compact_evidence(exit_code),
        ));
    }

    if command == b"dotnet" && matches!(arg1, b"build" | b"test" | b"format" | b"restore") {
        return Ok(StreamFilterOutput::new(
            compact_dotnet(stdout, stderr),
            Vec::new(),
            compact_evidence(exit_code),
        ));
    }
    if matches!(command, b"gradle" | b"gradlew")
        && (matches_gradle(stdout) || matches_gradle(stderr))
    {
        return Ok(StreamFilterOutput::new(
            compact_gradle(stdout, stderr),
            Vec::new(),
            compact_evidence(exit_code),
        ));
    }
    if matches!(command, b"mvn" | b"mvnw") && (matches_maven(stdout) || matches_maven(stderr)) {
        return Ok(StreamFilterOutput::new(
            compact_maven(stdout, stderr),
            Vec::new(),
            compact_evidence(exit_code),
        ));
    }
    if matches!(command, b"swift" | b"xcodebuild") {
        return Ok(StreamFilterOutput::new(
            compact_apple_build(stdout, stderr),
            Vec::new(),
            compact_evidence(exit_code),
        ));
    }
    if matches!(command, b"uv" | b"uvx") || runner_package_prelude {
        return Ok(StreamFilterOutput::new(
            compact_package_tool(stdout, stderr),
            Vec::new(),
            compact_evidence(exit_code),
        ));
    }

    Ok(passthrough(stdout, stderr))
}

pub(crate) fn has_package_prelude(input: &[u8]) -> bool {
    input.split(|byte| *byte == b'\n').any(|raw| {
        let line = trim_ascii_end(trim_ascii_start(raw));
        [
            b"Installed ".as_slice(),
            b"Resolved ",
            b"Prepared ",
            b"Downloaded ",
        ]
        .iter()
        .any(|prefix| {
            line.strip_prefix(*prefix)
                .is_some_and(|rest| find_subslice(rest, b" package").is_some())
        })
    })
}

fn compact_apple_build(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for input in [stdout, stderr] {
        scan_apple_build(input, &mut output);
    }
    if output.is_empty() && !stdout.is_empty() && stderr.is_empty() {
        b"ok\n".to_vec()
    } else {
        output
    }
}

fn scan_apple_build(input: &[u8], output: &mut Vec<u8>) {
    if input.is_empty() {
        return;
    }
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = trim_ascii_end(&clean);
        let trimmed = trim_ascii_start(line);
        if should_keep_apple_build(trimmed) {
            append_line(output, line);
        }
    }
}

fn should_keep_apple_build(line: &[u8]) -> bool {
    contains_ignore_ascii_case(line, b"error:")
        || contains_ignore_ascii_case(line, b"warning:")
        || [
            b"** BUILD FAILED **".as_slice(),
            b"** BUILD SUCCEEDED **",
            b"** TEST FAILED **",
            b"** TEST SUCCEEDED **",
            b"SwiftCompile",
            b"CompileSwift",
            b"Failing tests:",
            b"Test Suite",
            b"Executed ",
        ]
        .iter()
        .any(|needle| find_subslice(line, needle).is_some())
}

fn compact_package_tool(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for input in [stdout, stderr] {
        scan_package_tool(input, &mut output);
    }
    if output.is_empty() && !stdout.is_empty() && stderr.is_empty() {
        b"ok\n".to_vec()
    } else {
        output
    }
}

fn scan_package_tool(input: &[u8], output: &mut Vec<u8>) {
    if input.is_empty() {
        return;
    }
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = trim_ascii_end(&clean);
        if should_keep_package_tool(trim_ascii_start(line)) {
            append_line(output, line);
        }
    }
}

fn should_keep_package_tool(line: &[u8]) -> bool {
    if line.starts_with(b"Preparing packages") {
        return false;
    }
    let package_delta = matches!(line.first(), Some(b'+' | b'-'))
        && find_subslice(line.get(1..).unwrap_or_default(), b"==").is_some();
    package_delta
        || find_subslice(line, b"ERR!").is_some()
        || find_subslice(line, b"WARN").is_some()
        || [
            b"error".as_slice(),
            b"failed",
            b"deprecated",
            b"vulnerab",
            b"added ",
            b"removed ",
            b"changed ",
            b"packages",
            b"done in",
        ]
        .iter()
        .any(|needle| contains_ignore_ascii_case(line, needle))
        || line.starts_with(b"\xe2\x9c\x93")
        || line.starts_with(b"\xe2\x9c\x95")
}

fn matches_gradle(input: &[u8]) -> bool {
    [
        b"BUILD FAILED".as_slice(),
        b"BUILD SUCCESSFUL",
        b"FAILURE: Build failed",
        b"> Task ",
        b" tests completed",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

#[derive(Default)]
struct GradleState {
    in_cause_block: bool,
    after_failure_context: usize,
}

fn compact_gradle(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut state = GradleState::default();
    for input in [stdout, stderr] {
        scan_gradle(input, &mut output, &mut state);
    }
    if output.is_empty() {
        b"gradle ok\n".to_vec()
    } else {
        output
    }
}

fn scan_gradle(input: &[u8], output: &mut Vec<u8>, state: &mut GradleState) {
    if input.is_empty() {
        return;
    }
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = trim_ascii(&clean);
        if line.is_empty() {
            state.in_cause_block = false;
            state.after_failure_context = 0;
            continue;
        }
        if line.starts_with(b"* Try:") {
            state.in_cause_block = false;
            state.after_failure_context = 0;
            continue;
        }
        if should_keep_gradle(line) {
            append_line(output, line);
            if line == b"* What went wrong:" || line.starts_with(b"Execution failed") {
                state.in_cause_block = true;
            }
            if is_gradle_failure_header(line) || is_gradle_exception_line(line) {
                state.after_failure_context = 3;
            }
        } else if state.in_cause_block && is_gradle_cause_context(line) {
            append_line(output, line);
        } else if state.after_failure_context > 0 && is_gradle_failure_context(line) {
            append_line(output, line);
            state.after_failure_context -= 1;
        }
    }
}

fn should_keep_gradle(line: &[u8]) -> bool {
    line.starts_with(b"> Task ") && find_subslice(line, b"FAILED").is_some()
        || line.starts_with(b"FAILURE:")
        || line == b"* What went wrong:"
        || line.starts_with(b"Execution failed")
        || line.starts_with(b"> Compilation error")
        || line.starts_with(b"e: ")
        || line.starts_with(b"w: ")
        || line.starts_with(b"Note: ")
        || is_gradle_failure_header(line)
        || is_gradle_exception_line(line)
        || find_subslice(line, b" tests completed").is_some()
        || line.starts_with(b"There were failing tests.")
        || line.starts_with(b"BUILD FAILED")
        || line.starts_with(b"BUILD SUCCESSFUL")
        || line.starts_with(b"See the report at:")
        || line.starts_with(b"file://")
}

fn is_gradle_cause_context(line: &[u8]) -> bool {
    line.starts_with(b"> ")
        || line.starts_with(b"e: ")
        || line.starts_with(b"w: ")
        || find_subslice(line, b" error").is_some()
        || find_subslice(line, b"Error").is_some()
}

fn is_gradle_failure_header(line: &[u8]) -> bool {
    find_subslice(line, b" FAILED").is_some() || line.starts_with(b"FAILED ")
}

fn is_gradle_exception_line(line: &[u8]) -> bool {
    find_subslice(line, b"Exception").is_some()
        || find_subslice(line, b"AssertionError").is_some()
        || find_subslice(line, b"Error:").is_some()
}

fn is_gradle_failure_context(line: &[u8]) -> bool {
    line.starts_with(b"at ")
        && [b".kt:".as_slice(), b".java:", b".groovy:", b".scala:"]
            .iter()
            .any(|needle| find_subslice(line, needle).is_some())
        || line.starts_with(b"Caused by:")
        || find_subslice(line, b"expected:<").is_some()
}

fn matches_maven(input: &[u8]) -> bool {
    [
        b"[ERROR]".as_slice(),
        b"[WARNING]",
        b"BUILD FAILURE",
        b"BUILD SUCCESS",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

fn compact_maven(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut error_context = 0usize;
    for input in [stdout, stderr] {
        if input.is_empty() {
            continue;
        }
        for raw in input.split(|byte| *byte == b'\n') {
            let clean = strip_ansi(raw);
            let line = trim_ascii(&clean);
            if line.is_empty() {
                error_context = 0;
                continue;
            }
            if should_keep_maven(line) {
                let emitted = if line.starts_with(b"[INFO] BUILD ") {
                    &line[b"[INFO] ".len()..]
                } else {
                    line
                };
                append_line(&mut output, emitted);
                error_context = usize::from(line.starts_with(b"[ERROR]")) * 4;
            } else if error_context > 0 && is_maven_error_continuation(line) {
                append_line(&mut output, line);
                error_context -= 1;
            }
        }
    }
    if output.is_empty() {
        b"maven ok\n".to_vec()
    } else {
        output
    }
}

fn should_keep_maven(line: &[u8]) -> bool {
    line.starts_with(b"[ERROR]")
        || line.starts_with(b"[WARNING]")
        || matches!(line, b"[INFO] BUILD FAILURE" | b"[INFO] BUILD SUCCESS")
        || line.starts_with(b"[INFO] Total time:")
}

fn is_maven_error_continuation(line: &[u8]) -> bool {
    !line.starts_with(b"[")
        && (line.starts_with(b"symbol:")
            || line.starts_with(b"location:")
            || line.starts_with(b"required:")
            || line.starts_with(b"found:")
            || line.starts_with(b"reason:")
            || find_subslice(line, b"cannot find symbol").is_some())
}

fn compact_dotnet(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    for input in [stdout, stderr] {
        for raw in input.split(|byte| *byte == b'\n') {
            if raw.is_empty() {
                continue;
            }
            let clean = strip_ansi(raw);
            let line = trim_ascii_end(&clean);
            if should_keep_dotnet(line) {
                append_line(&mut output, line);
            }
        }
    }
    output
}

fn should_keep_dotnet(line: &[u8]) -> bool {
    let trimmed = trim_ascii(line);
    if trimmed.is_empty() {
        return false;
    }
    contains_ignore_ascii_case(trimmed, b": error ")
        || contains_ignore_ascii_case(trimmed, b": warning ")
        || find_subslice(trimmed, b" error CS").is_some()
        || find_subslice(trimmed, b" warning CS").is_some()
        || contains_ignore_ascii_case(trimmed, b"build failed")
        || contains_ignore_ascii_case(trimmed, b"build succeeded")
        || trimmed.ends_with(b" Error(s)")
        || trimmed.ends_with(b" Warning(s)")
        || trimmed.starts_with(b"Restored ")
        || contains_ignore_ascii_case(trimmed, b"restore failed")
        || contains_ignore_ascii_case(trimmed, b"restore succeeded")
        || contains_ignore_ascii_case(trimmed, b"test run failed")
        || trimmed.starts_with(b"[xUnit.net ") && find_subslice(trimmed, b"[FAIL]").is_some()
        || trimmed.starts_with(b"Failed ")
        || contains_ignore_ascii_case(trimmed, b"error message:")
        || find_subslice(trimmed, b"Assert.").is_some()
        || trimmed.starts_with(b"Expected:")
        || trimmed.starts_with(b"Actual:")
        || contains_ignore_ascii_case(trimmed, b"stack trace:")
        || trimmed.starts_with(b"at ") && find_subslice(trimmed, b".cs:line ").is_some()
        || contains_ignore_ascii_case(trimmed, b"failed!")
        || contains_ignore_ascii_case(trimmed, b"passed!")
        || contains_ignore_ascii_case(trimmed, b"failed:")
        || contains_ignore_ascii_case(trimmed, b"passed:")
        || contains_ignore_ascii_case(trimmed, b"total tests:")
        || contains_ignore_ascii_case(trimmed, b"failed tests:")
        || contains_ignore_ascii_case(trimmed, b"format complete")
        || contains_ignore_ascii_case(trimmed, b"formatted code file")
        || contains_ignore_ascii_case(trimmed, b"would be formatted")
}

fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

fn compact_evidence(exit_code: i32) -> EvidenceClass {
    if exit_code == 0 {
        EvidenceClass::PotentiallyLossy
    } else {
        EvidenceClass::FactComplete
    }
}

fn has_recognized_failure(input: &[u8]) -> bool {
    [
        b"error:".as_slice(),
        b"Error:",
        b": error ",
        b" error CS",
        b"[ERROR]",
        b"ERROR ",
        b"FAILURE:",
        b"BUILD FAILED",
        b"BUILD FAILURE",
        b"Build FAILED",
        b"Test run failed",
        b"[FAIL]",
        b"Failed!",
        b" FAILED",
        b"Exception",
        b"AssertionError",
        b"** BUILD FAILED **",
        b"** TEST FAILED **",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

fn matches_build_output(input: &[u8]) -> bool {
    find_subslice(input, b"Tasks:").is_some()
        && find_subslice(input, b"Duration:").is_some()
        && (find_subslice(input, b"\n> ").is_some() || input.starts_with(b"> "))
        || find_subslice(input, b"vite v").is_some()
            && (find_subslice(input, b"building for production").is_some()
                || find_subslice(input, b"building SSR bundle").is_some())
        || find_subslice(input, b"\xe2\x96\xb2 Next.js").is_some()
        || find_subslice(input, b"Creating an optimized production build").is_some()
        || find_subslice(input, b"Compiled successfully").is_some()
        || find_subslice(input, b"Nuxt ").is_some() && find_subslice(input, b"with Nitro").is_some()
        || find_subslice(input, b"webpack ").is_some()
            && find_subslice(input, b" compiled ").is_some()
        || find_subslice(input, b"modules transformed").is_some()
        || find_subslice(input, b"\xe2\x9c\x93 built in ").is_some()
        || find_subslice(input, b"\xce\xa3 Total size:").is_some()
}

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

fn compact_build_output(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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
    output.extend_from_slice(b"(smll: omitted ");
    output.extend_from_slice(omitted.to_string().as_bytes());
    output.extend_from_slice(b" relevant lines; rerun with smll --raw)\n");
    output.extend_from_slice(&data[tail_start..]);
    output
}

fn byte_after_lines(data: &[u8], line_count: usize) -> usize {
    if line_count == 0 {
        return 0;
    }
    let mut seen = 0usize;
    for (index, byte) in data.iter().enumerate() {
        if *byte == b'\n' {
            seen += 1;
            if seen == line_count {
                return index + 1;
            }
        }
    }
    data.len()
}

fn matches_build_compact(stdout: &[u8], stderr: &[u8]) -> bool {
    [stdout, stderr]
        .into_iter()
        .flat_map(|input| input.split(|byte| *byte == b'\n'))
        .any(|line| {
            line.starts_with(b"Build Summary: ") || classify_build_line(line) != BuildLine::Other
        })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum BuildLine {
    CargoProgress,
    CargoCheckProgress,
    CargoVerboseInvocation,
    MakeProgress,
    NinjaProgress(usize),
    GoProgress,
    Kept,
    Other,
}

fn classify_build_line(line: &[u8]) -> BuildLine {
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

fn compact_build(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if let Some(summary) =
        find_zig_success_summary(stdout).or_else(|| find_zig_success_summary(stderr))
    {
        let mut output = summary.to_vec();
        output.push(b'\n');
        return output;
    }

    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut cargo_count = 0usize;
    let mut cargo_verbose_count = 0usize;
    let mut make_count = 0usize;
    let mut ninja_count = 0usize;
    let mut go_count = 0usize;
    let mut cargo_finished_dev = false;
    for input in [stdout, stderr] {
        if input.is_empty() {
            continue;
        }
        let input = input.strip_suffix(b"\n").unwrap_or(input);
        for raw in input.split(|byte| *byte == b'\n') {
            let clean = strip_ansi(raw);
            let line = clean.as_slice();
            if is_make_directory_noise(line) {
                continue;
            }
            match classify_build_line(line) {
                BuildLine::CargoProgress | BuildLine::CargoCheckProgress => cargo_count += 1,
                BuildLine::CargoVerboseInvocation => cargo_verbose_count += 1,
                BuildLine::MakeProgress => make_count += 1,
                BuildLine::NinjaProgress(completed) => {
                    ninja_count = ninja_count.max(completed);
                }
                BuildLine::GoProgress => go_count += 1,
                BuildLine::Kept | BuildLine::Other => {
                    if line.starts_with(b"    Finished dev") {
                        cargo_finished_dev = true;
                    } else if !is_cargo_generated_warning_summary(line) {
                        append_line(&mut output, line);
                    }
                }
            }
        }
    }
    if cargo_count > 0 {
        output.extend_from_slice(b"cargo: ");
        if cargo_finished_dev {
            output.extend_from_slice(b"Finished dev; ");
        }
        output.extend_from_slice(cargo_count.to_string().as_bytes());
        output.extend_from_slice(b" crates\n");
    } else if cargo_verbose_count > 0 {
        output.extend_from_slice(b"Ran ");
        output.extend_from_slice(cargo_verbose_count.to_string().as_bytes());
        output.extend_from_slice(b" rustc invocations (cargo -vv)\n");
    }
    if make_count > 0 {
        output.extend_from_slice(b"Compiled ");
        output.extend_from_slice(make_count.to_string().as_bytes());
        output.extend_from_slice(b" (make)\n");
    }
    if ninja_count > 0 {
        output.extend_from_slice(b"built ");
        output.extend_from_slice(ninja_count.to_string().as_bytes());
        output.extend_from_slice(b" (ninja)\n");
    }
    if go_count > 0 {
        output.extend_from_slice(b"Compiled ");
        output.extend_from_slice(go_count.to_string().as_bytes());
        output.extend_from_slice(b" (go)\n");
    }
    output
}

fn ninja_completed(line: &[u8]) -> Option<usize> {
    if line.len() < 6 || line[0] != b'[' {
        return None;
    }
    let mut index = 1usize;
    let mut completed = 0usize;
    while line.get(index).is_some_and(u8::is_ascii_digit) {
        completed = completed * 10 + usize::from(line[index] - b'0');
        index += 1;
    }
    if index == 1 || line.get(index) != Some(&b'/') {
        return None;
    }
    index += 1;
    let total_start = index;
    while line.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if index == total_start || line.get(index) != Some(&b']') || line.get(index + 1) != Some(&b' ')
    {
        return None;
    }
    Some(completed)
}

fn find_zig_success_summary(input: &[u8]) -> Option<&[u8]> {
    let start = find_subslice(input, b"Build Summary: ")?;
    let rest = &input[start..];
    let end = rest
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(rest.len());
    let line = &rest[..end];
    find_subslice(line, b"failed").is_none().then_some(line)
}

fn is_make_directory_noise(line: &[u8]) -> bool {
    line.starts_with(b"make")
        && (find_subslice(line, b": Entering directory").is_some()
            || find_subslice(line, b": Leaving directory").is_some())
}

fn is_cargo_generated_warning_summary(line: &[u8]) -> bool {
    line.starts_with(b"warning: `") && find_subslice(line, b" generated ").is_some()
}

fn requests_exact_output(argv: &[&[u8]]) -> bool {
    for argument in &argv[1..] {
        if *argument == b"--" {
            break;
        }
        if matches!(*argument, b"--help" | b"--version" | b"-h" | b"-V") {
            return true;
        }
        if is_dotnet_query_switch(argument) {
            return true;
        }
    }
    matches!(argv.get(1), Some(&b"help") | Some(&b"version"))
}

fn is_dotnet_query_switch(argument: &[u8]) -> bool {
    let rest = if let Some(rest) = argument.strip_prefix(b"--") {
        rest
    } else if matches!(argument.first(), Some(b'-' | b'/')) {
        &argument[1..]
    } else {
        return false;
    };
    [b"getproperty".as_slice(), b"getitem", b"gettargetresult"]
        .iter()
        .any(|query| {
            rest.get(..query.len())
                .is_some_and(|name| name.eq_ignore_ascii_case(query))
                && rest.get(query.len()) == Some(&b':')
        })
}

fn command_basename(command: &[u8]) -> &[u8] {
    command
        .iter()
        .rposition(|byte| matches!(byte, b'/' | b'\\'))
        .map_or(command, |separator| &command[separator + 1..])
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn append_line(output: &mut Vec<u8>, line: &[u8]) {
    output.extend_from_slice(line);
    output.push(b'\n');
}

fn trim_ascii_start(mut input: &[u8]) -> &[u8] {
    while input
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[1..];
    }
    input
}

fn trim_ascii_end(mut input: &[u8]) -> &[u8] {
    while input
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[..input.len() - 1];
    }
    input
}

fn trim_ascii(input: &[u8]) -> &[u8] {
    trim_ascii_end(trim_ascii_start(input))
}

fn strip_ansi(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0usize;
    while index < input.len() {
        if input[index] != 0x1b {
            output.push(input[index]);
            index += 1;
            continue;
        }
        match input.get(index + 1) {
            Some(b'[') => {
                index += 2;
                while index < input.len() && !(0x40..=0x7e).contains(&input[index]) {
                    index += 1;
                }
                index += usize::from(index < input.len());
            }
            Some(b']') => {
                index += 2;
                while index < input.len() {
                    if input[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if input[index] == 0x1b && input.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    output
}

fn passthrough(stdout: &[u8], stderr: &[u8]) -> StreamFilterOutput {
    StreamFilterOutput::new(stdout.to_vec(), stderr.to_vec(), EvidenceClass::ByteExact)
}
