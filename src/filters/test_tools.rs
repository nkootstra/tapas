use std::collections::VecDeque;

use super::{
    EvidenceClass, FilterError, FilterOutput, StreamFilterOutput, command_basename, find_subslice,
    strip_ansi,
};

type Matcher = fn(&[u8]) -> bool;
type Apply = fn(&[u8], &[u8]) -> Vec<u8>;

const PIPE_FILTERS: &[(Matcher, Apply)] = &[
    (matches_cargo_test, apply_cargo_test),
    (matches_jest, apply_jest),
    (matches_js_test, apply_js_test),
    (matches_tsc, apply_tsc),
    (matches_go_test, apply_go_test),
    (matches_pytest, apply_pytest),
];

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    let Some(command) = argv.first().copied().map(command_basename) else {
        return false;
    };
    let arg1 = argv.get(1).copied().unwrap_or_default();
    matches!(command, b"pytest" | b"jest" | b"vitest" | b"mocha" | b"tsc")
        || command == b"cargo" && arg1 == b"test"
        || command == b"go" && arg1 == b"test"
        || command == b"node" && arg1 == b"--test"
        || matches!(command, b"npm" | b"pnpm" | b"yarn" | b"bun") && arg1 == b"test"
}

pub fn matches(input: &[u8]) -> bool {
    PIPE_FILTERS.iter().any(|(matches, _)| matches(input))
}

pub fn apply_matched(input: &[u8]) -> Result<FilterOutput, FilterError> {
    PIPE_FILTERS
        .iter()
        .find(|(matches, _)| matches(input))
        .map(|(_, apply)| FilterOutput::new(apply(input, b""), EvidenceClass::FactComplete))
        .ok_or(FilterError::InvalidInput)
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
    if requests_exact_output(command, argv) {
        return Ok(StreamFilterOutput::passthrough(stdout, stderr));
    }
    let arg1 = argv.get(1).copied().unwrap_or_default();
    let script_test = arg1 == b"test" && matches!(command, b"npm" | b"pnpm" | b"yarn" | b"bun");

    let compact = if command == b"pytest" && stream_matches(stdout, stderr, matches_pytest) {
        Some(apply_pytest(stdout, stderr))
    } else if command == b"cargo"
        && arg1 == b"test"
        && stream_matches(stdout, stderr, matches_cargo_test)
    {
        Some(apply_cargo_test(stdout, stderr))
    } else if (matches!(command, b"jest" | b"vitest") || script_test)
        && stream_matches(stdout, stderr, matches_jest)
    {
        Some(apply_jest(stdout, stderr))
    } else if (script_test || command == b"mocha" || (command == b"node" && arg1 == b"--test"))
        && stream_matches(stdout, stderr, matches_js_test)
    {
        Some(apply_js_test(stdout, stderr))
    } else if command == b"tsc" && stream_matches(stdout, stderr, matches_tsc) {
        Some(apply_tsc(stdout, stderr))
    } else if command == b"go" && arg1 == b"test" && stream_matches(stdout, stderr, matches_go_test)
    {
        Some(apply_go_test(stdout, stderr))
    } else {
        None
    };

    Ok(compact.map_or_else(
        || StreamFilterOutput::passthrough(stdout, stderr),
        |stdout| StreamFilterOutput::new(stdout, Vec::new(), EvidenceClass::FactComplete),
    ))
}

fn stream_matches(stdout: &[u8], stderr: &[u8], matcher: fn(&[u8]) -> bool) -> bool {
    matcher(stdout) || matcher(stderr)
}

fn requests_exact_output(command: &[u8], argv: &[&[u8]]) -> bool {
    for argument in &argv[1..] {
        if *argument == b"--" {
            break;
        }
        if matches!(*argument, b"--help" | b"--version") {
            return true;
        }
        if matches!(*argument, b"-h" | b"-V") && command == b"pytest" {
            return true;
        }
    }
    if matches!(argv.get(1), Some(&b"help") | Some(&b"version")) {
        return true;
    }
    if command == b"pytest"
        && argv[1..].iter().any(|argument| {
            matches!(
                *argument,
                b"--collect-only"
                    | b"--co"
                    | b"--fixtures"
                    | b"--fixtures-per-test"
                    | b"--markers"
                    | b"--trace-config"
            )
        })
    {
        return true;
    }
    command == b"tsc"
        && argv[1..]
            .iter()
            .any(|argument| matches!(*argument, b"--showConfig" | b"--listFilesOnly"))
}

fn matches_cargo_test(input: &[u8]) -> bool {
    if find_subslice(input, b"test result:").is_some() {
        return true;
    }
    input.split(|byte| *byte == b'\n').any(|line| {
        let line = line.trim_ascii_start();
        line.starts_with(b"running ") && find_subslice(line, b" test").is_some()
    })
}

fn matches_jest(input: &[u8]) -> bool {
    find_subslice(input, b"Test Suites:").is_some()
        || find_subslice(input, b"Test Files").is_some()
        || input.starts_with(b"Tests:")
        || find_subslice(input, b"\nTests:").is_some()
}

fn apply_jest(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_jest(stdout, &mut output);
    scan_jest(stderr, &mut output);
    if output.is_empty() || !has_jest_failure(&output) {
        b"all tests passed\n".to_vec()
    } else {
        head_tail(output, 120, 80)
    }
}

fn scan_jest(input: &[u8], output: &mut Vec<u8>) {
    let mut in_error_context = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii();
        if line.is_empty() {
            in_error_context = false;
            continue;
        }
        if line.starts_with(b"PASS ") || line.starts_with(b"PASS\t") {
            in_error_context = false;
            continue;
        }
        if line.starts_with("✓ ".as_bytes())
            || line.starts_with(b"Snapshots:")
            || line.starts_with(b"Time:")
            || line.starts_with(b"Ran all")
        {
            continue;
        }
        if should_keep_jest(line) {
            in_error_context = true;
            write_line(output, line);
        } else if in_error_context {
            write_line(output, line);
        }
    }
}

fn should_keep_jest(line: &[u8]) -> bool {
    [
        b"FAIL ".as_slice(),
        b"FAIL\t",
        "● ".as_bytes(),
        b"Expected:",
        b"Received:",
        b"expect(",
        b"Error:",
        b"    at ",
        b"Test Suites:",
        b"Tests:",
        b"Test Files",
        "✗ ".as_bytes(),
        "✕ ".as_bytes(),
        b" failed",
        b"Failed",
        b"    > ",
        b">   ",
        b"E   ",
    ]
    .iter()
    .any(|needle| find_subslice(line, needle).is_some())
}

fn has_jest_failure(input: &[u8]) -> bool {
    if find_subslice(input, b"FAIL").is_some() || find_subslice(input, "●".as_bytes()).is_some() {
        return true;
    }
    input
        .split(|byte| *byte == b'\n')
        .any(|line| nonzero_count_before(line, b"failed"))
}

fn nonzero_count_before(line: &[u8], marker: &[u8]) -> bool {
    let Some(end) = find_subslice(line, marker) else {
        return false;
    };
    if end < 2 || line[end - 1] != b' ' {
        return false;
    }
    let mut start = end - 1;
    while start > 0 && line[start - 1].is_ascii_digit() {
        start -= 1;
    }
    start < end - 1 && line[start..end - 1].iter().any(|byte| *byte != b'0')
}

#[derive(Clone, Copy)]
enum JsTestMode {
    Mocha,
    Node,
}

fn matches_js_test(input: &[u8]) -> bool {
    matches_mocha(input) || matches_node_test(input)
}

fn matches_mocha(input: &[u8]) -> bool {
    input.split(|byte| *byte == b'\n').any(|line| {
        let line = line.trim_ascii();
        is_number_summary(line, b" passing") || is_number_summary(line, b" failing")
    })
}

fn matches_node_test(input: &[u8]) -> bool {
    let mut has_tests = false;
    let mut has_result = false;
    for line in input.split(|byte| *byte == b'\n') {
        let line = line.trim_ascii();
        has_tests |= line.starts_with(b"# tests ");
        has_result |= line.starts_with(b"# fail ")
            || line.starts_with(b"# pass ")
            || line.starts_with(b"not ok ")
            || starts_with_unicode_failure(line);
    }
    has_tests && has_result
}

fn apply_js_test(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mode = if matches_node_test(stdout) || matches_node_test(stderr) {
        JsTestMode::Node
    } else {
        JsTestMode::Mocha
    };
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    match mode {
        JsTestMode::Mocha => {
            scan_mocha(stdout, &mut output);
            scan_mocha(stderr, &mut output);
        }
        JsTestMode::Node => {
            scan_node_test(stdout, &mut output);
            scan_node_test(stderr, &mut output);
        }
    }
    if output.is_empty() {
        b"all tests passed\n".to_vec()
    } else {
        output
    }
}

fn scan_mocha(input: &[u8], output: &mut Vec<u8>) {
    let input = input.strip_suffix(b"\n").unwrap_or(input);
    if input.is_empty() {
        return;
    }
    let mut in_failure = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii_end();
        let trimmed = line.trim_ascii();
        if starts_with_unicode_pass(trimmed) {
            continue;
        }
        if is_number_summary(trimmed, b" passing") || is_number_summary(trimmed, b" failing") {
            in_failure = false;
            write_line(output, trimmed);
        } else if is_mocha_failure_header(trimmed) {
            in_failure = true;
            write_line(output, trimmed);
        } else if in_failure {
            write_line(output, line);
        }
    }
}

fn scan_node_test(input: &[u8], output: &mut Vec<u8>) {
    let input = input.strip_suffix(b"\n").unwrap_or(input);
    if input.is_empty() {
        return;
    }
    let mut in_failure = false;
    let mut skipping_pass = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii_end();
        let trimmed = line.trim_ascii();
        if skipping_pass {
            if trimmed == b"..." {
                skipping_pass = false;
            }
            continue;
        }
        if starts_with_unicode_pass(trimmed) {
            continue;
        }
        if trimmed.starts_with(b"ok ") {
            skipping_pass = true;
            continue;
        }
        if trimmed.starts_with(b"TAP version ") {
            continue;
        }
        if is_node_trailer(trimmed) {
            in_failure = false;
            write_line(output, trimmed);
        } else if trimmed.starts_with(b"not ok ") || starts_with_unicode_failure(trimmed) {
            in_failure = true;
            write_line(output, trimmed);
        } else if in_failure {
            write_line(output, line);
            if trimmed == b"..." {
                in_failure = false;
            }
        }
    }
}

fn is_number_summary(line: &[u8], marker: &[u8]) -> bool {
    let Some(position) = find_subslice(line, marker) else {
        return false;
    };
    if position == 0 || !line[..position].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let after = &line[position + marker.len()..];
    after.is_empty() || (after.len() >= 2 && after[0] == b' ' && after[1] == b'(')
}

fn is_mocha_failure_header(line: &[u8]) -> bool {
    let digits = line
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(line.len());
    digits > 0 && line.get(digits..digits + 2) == Some(b") ")
}

fn starts_with_unicode_pass(line: &[u8]) -> bool {
    line.starts_with("✔ ".as_bytes()) || line.starts_with("✓ ".as_bytes())
}

fn starts_with_unicode_failure(line: &[u8]) -> bool {
    line.starts_with("✖ ".as_bytes()) || line.starts_with("✕ ".as_bytes())
}

fn is_node_trailer(line: &[u8]) -> bool {
    line.starts_with(b"1..")
        || [
            b"# tests ".as_slice(),
            b"# suites ",
            b"# pass ",
            b"# fail ",
            b"# cancelled ",
            b"# skipped ",
            b"# todo ",
            b"# duration_ms ",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

#[derive(Debug)]
struct TscDiagnostic {
    location: Vec<u8>,
    rest: Vec<u8>,
    code: Vec<u8>,
    message: Vec<u8>,
}

fn matches_tsc(input: &[u8]) -> bool {
    find_subslice(input, b"error TS").is_some()
        || find_subslice(input, b"Found 0 errors").is_some()
        || (find_subslice(input, b"Found ").is_some()
            && find_subslice(input, b" errors in ").is_some())
}

fn apply_tsc(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut diagnostics = Vec::new();
    let mut raw_lines = Vec::new();
    let mut summaries = Vec::new();
    collect_tsc(stdout, &mut diagnostics, &mut raw_lines, &mut summaries);
    collect_tsc(stderr, &mut diagnostics, &mut raw_lines, &mut summaries);
    if diagnostics.is_empty() && raw_lines.is_empty() && summaries.is_empty() {
        return b"no type errors\n".to_vec();
    }

    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    emit_tsc_diagnostics(&diagnostics, &mut output);
    for line in raw_lines.iter().chain(summaries.iter()) {
        write_line(&mut output, line);
    }
    output
}

fn collect_tsc(
    input: &[u8],
    diagnostics: &mut Vec<TscDiagnostic>,
    raw_lines: &mut Vec<Vec<u8>>,
    summaries: &mut Vec<Vec<u8>>,
) {
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if is_tsc_summary(line) {
            summaries.push(line.to_vec());
            continue;
        }
        if let Some(position) = find_subslice(line, b" - error TS") {
            let rest = &line[position + b" - error ".len()..];
            if let Some(colon) = rest.iter().position(|byte| *byte == b':') {
                diagnostics.push(TscDiagnostic {
                    location: line[..position].to_vec(),
                    rest: rest.to_vec(),
                    code: rest[..colon].to_vec(),
                    message: rest[colon + 1..].trim_ascii().to_vec(),
                });
            } else {
                raw_lines.push(line.to_vec());
            }
        } else if find_subslice(line, b"error TS").is_some() {
            raw_lines.push(line.to_vec());
        }
    }
}

fn emit_tsc_diagnostics(diagnostics: &[TscDiagnostic], output: &mut Vec<u8>) {
    let mut emitted = vec![false; diagnostics.len()];
    for index in 0..diagnostics.len() {
        if emitted[index] {
            continue;
        }
        let group: Vec<usize> = diagnostics
            .iter()
            .enumerate()
            .filter_map(|(candidate, diagnostic)| {
                (diagnostic.code == diagnostics[index].code).then_some(candidate)
            })
            .collect();
        for &candidate in &group {
            emitted[candidate] = true;
        }
        let key = message_key(&diagnostics[group[0]].message);
        let homogeneous = group
            .iter()
            .skip(1)
            .all(|candidate| message_key(&diagnostics[*candidate].message) == key);
        if group.len() >= 3 && homogeneous {
            output.extend_from_slice(&diagnostics[index].code);
            output.extend_from_slice(b" x");
            output.extend_from_slice(group.len().to_string().as_bytes());
            output.extend_from_slice(b": ");
            for (position, candidate) in group.iter().take(3).enumerate() {
                if position > 0 {
                    output.extend_from_slice(b", ");
                }
                output.extend_from_slice(&diagnostics[*candidate].location);
            }
            if group.len() > 3 {
                output.extend_from_slice(b", ... (");
                output.extend_from_slice((group.len() - 3).to_string().as_bytes());
                output.extend_from_slice(b" more)");
            }
            output.push(b'\n');
            if !diagnostics[index].message.is_empty() {
                write_line(output, &diagnostics[index].message);
            }
        } else {
            for candidate in group {
                output.extend_from_slice(&diagnostics[candidate].location);
                output.push(b' ');
                write_line(output, &diagnostics[candidate].rest);
            }
        }
    }
}

fn message_key(message: &[u8]) -> &[u8] {
    &message[..message.len().min(40)]
}

fn is_tsc_summary(line: &[u8]) -> bool {
    line.starts_with(b"Found ") && find_subslice(line, b"error").is_some()
}

fn matches_go_test(input: &[u8]) -> bool {
    find_subslice(input, b"=== RUN").is_some()
        || find_subslice(input, b"--- FAIL:").is_some()
        || find_subslice(input, b"--- PASS:").is_some()
        || input.starts_with(b"Benchmark")
        || find_subslice(input, b"\nBenchmark").is_some()
        || find_subslice(input, b"=== FUZZ").is_some()
        || input.starts_with(b"ok  \t")
        || find_subslice(input, b"\nok  \t").is_some()
        || input.starts_with(b"FAIL\t")
        || find_subslice(input, b"\nFAIL\t").is_some()
}

fn apply_go_test(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut has_benchmark_or_fuzz = false;
    scan_go_test(stdout, &mut output, &mut has_benchmark_or_fuzz);
    scan_go_test(stderr, &mut output, &mut has_benchmark_or_fuzz);
    let has_failure = find_subslice(&output, b"--- FAIL:").is_some()
        || find_subslice(&output, b"FAIL\t").is_some();
    if has_benchmark_or_fuzz || has_failure {
        head_tail(output, 120, 80)
    } else {
        b"all tests passed\n".to_vec()
    }
}

fn scan_go_test(input: &[u8], output: &mut Vec<u8>, has_benchmark_or_fuzz: &mut bool) {
    let mut pending = Vec::new();
    let mut last_fuzz_progress = Vec::new();
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if is_go_benchmark(line) {
            output.append(&mut pending);
            write_line(output, line);
            *has_benchmark_or_fuzz = true;
            continue;
        }
        if line.starts_with(b"fuzz: ") {
            last_fuzz_progress.clear();
            write_line(&mut last_fuzz_progress, line);
            *has_benchmark_or_fuzz = true;
            continue;
        }
        if line.starts_with(b"--- FUZZ:") || line.starts_with(b"=== FUZZ") {
            output.append(&mut pending);
            write_line(output, line);
            *has_benchmark_or_fuzz = true;
            continue;
        }
        if line.starts_with(b"--- FAIL:") {
            output.append(&mut pending);
            write_line(output, line);
            continue;
        }
        if line.starts_with(b"--- PASS:") || line.starts_with(b"--- SKIP:") {
            pending.clear();
            continue;
        }
        if line.starts_with(b"=== ") {
            pending.clear();
            continue;
        }
        if raw.starts_with(b"    ") || raw.starts_with(b"\t") {
            write_line(&mut pending, line);
            continue;
        }
        output.append(&mut pending);
        if line.starts_with(b"FAIL\t") || line.starts_with(b"ok\t") || line.starts_with(b"ok  ") {
            write_line(output, line);
        }
    }
    output.extend_from_slice(&last_fuzz_progress);
}

fn is_go_benchmark(line: &[u8]) -> bool {
    line.starts_with(b"Benchmark") && line.contains(&b'\t')
}

fn matches_pytest(input: &[u8]) -> bool {
    if find_subslice(input, b"test session starts").is_some()
        || find_subslice(input, b"passed in ").is_some()
        || find_subslice(input, b"failed in ").is_some()
    {
        return true;
    }
    if find_subslice(input, b"collected ").is_some() {
        return input.split(|byte| *byte == b'\n').any(|line| {
            find_subslice(line, b"collected ").is_some() && find_subslice(line, b" item").is_some()
        });
    }
    false
}

fn apply_pytest(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_pytest(stdout, &mut output);
    scan_pytest(stderr, &mut output);
    if output.is_empty() || !has_pytest_failure(&output) {
        b"all tests passed\n".to_vec()
    } else {
        head_tail(output, 120, 80)
    }
}

fn scan_pytest(input: &[u8], output: &mut Vec<u8>) {
    let mut in_error_context = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii();
        if line.is_empty() {
            in_error_context = false;
            continue;
        }
        if should_keep_pytest(line) {
            in_error_context = true;
            let without_frame = trim_bytes(line, |byte| matches!(byte, b'=' | b' '));
            write_line(
                output,
                if without_frame.is_empty() {
                    line
                } else {
                    without_frame
                },
            );
        } else if in_error_context {
            write_line(output, line);
        }
    }
}

fn should_keep_pytest(line: &[u8]) -> bool {
    [
        b"FAILED".as_slice(),
        b"ERROR",
        b"failed",
        b"error",
        b"assert",
        b"collected",
        b"short test summary",
        b"==== ",
        b">   ",
        b"E   ",
    ]
    .iter()
    .any(|needle| find_subslice(line, needle).is_some())
}

fn has_pytest_failure(input: &[u8]) -> bool {
    find_subslice(input, b"FAILED").is_some()
        || find_subslice(input, b"ERROR").is_some()
        || input
            .split(|byte| *byte == b'\n')
            .any(|line| nonzero_count_before(line, b"failed"))
}

fn trim_bytes(input: &[u8], predicate: impl Fn(u8) -> bool) -> &[u8] {
    let start = input
        .iter()
        .position(|byte| !predicate(*byte))
        .unwrap_or(input.len());
    let end = input
        .iter()
        .rposition(|byte| !predicate(*byte))
        .map_or(start, |position| position + 1);
    &input[start..end]
}

fn apply_cargo_test(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_cargo_test(stdout, &mut output);
    scan_cargo_test(stderr, &mut output);
    if output.is_empty() {
        b"all tests passed\n".to_vec()
    } else {
        head_tail(output, 120, 80)
    }
}

fn scan_cargo_test(input: &[u8], output: &mut Vec<u8>) {
    let mut before = VecDeque::<Vec<u8>>::with_capacity(3);
    let mut in_error_context = false;
    let mut after_remaining = 0;
    let mut dropping_names = false;

    for raw in input.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.trim_ascii();
        if line.is_empty() {
            in_error_context = false;
            after_remaining = 0;
            dropping_names = false;
            continue;
        }
        if line.starts_with(b"note: run with") {
            continue;
        }
        if line == b"failures:" {
            dropping_names = true;
            continue;
        }
        if dropping_names {
            if raw.starts_with(b"    ") || raw.starts_with(b"\t") {
                continue;
            }
            dropping_names = false;
        }

        if should_keep_cargo(line) {
            for context in before.drain(..) {
                write_line(output, &context);
            }
            in_error_context = line.starts_with(b"error") || line.starts_with(b"warning");
            after_remaining = 3;
            if line.starts_with(b"test result:") {
                write_cargo_result(output, line);
                in_error_context = false;
                after_remaining = 0;
            } else {
                write_line(output, line);
            }
            continue;
        }
        if in_error_context && is_cargo_error_context(line) {
            write_line(output, line);
            continue;
        }
        if after_remaining > 0 {
            write_line(output, line);
            after_remaining -= 1;
            continue;
        }
        in_error_context = false;
        if before.len() == 3 {
            before.pop_front();
        }
        before.push_back(line.to_vec());
    }
}

fn should_keep_cargo(line: &[u8]) -> bool {
    if line.starts_with(b"error: test failed, to rerun pass") {
        return false;
    }
    [
        b"error[".as_slice(),
        b"error:",
        b"warning:",
        b"test result:",
        b"panicked at",
        b"---- ",
        b"bench:",
    ]
    .iter()
    .any(|needle| find_subslice(line, needle).is_some())
}

fn is_cargo_error_context(line: &[u8]) -> bool {
    line.starts_with(b"-->")
        || line.starts_with(b"= ")
        || line.first().is_some_and(u8::is_ascii_digit)
        || line.starts_with(b"|")
        || line.starts_with(b"^")
        || line.starts_with(b"-")
        || line.starts_with(b"For more info")
}

fn write_cargo_result(output: &mut Vec<u8>, line: &[u8]) {
    output.extend_from_slice(b"res ");
    output.extend_from_slice(number_before(line, b" passed").unwrap_or(b"0"));
    output.extend_from_slice(b"p ");
    output.extend_from_slice(number_before(line, b" failed").unwrap_or(b"0"));
    output.push(b'f');
    if let Some(position) = find_subslice(line, b"finished in ") {
        let duration = first_token(&line[position + b"finished in ".len()..]);
        if !duration.is_empty() {
            output.push(b' ');
            output.extend_from_slice(duration);
        }
    }
    output.push(b'\n');
}

fn number_before<'a>(line: &'a [u8], marker: &[u8]) -> Option<&'a [u8]> {
    let end = find_subslice(line, marker)?;
    let mut start = end;
    while start > 0 && line[start - 1].is_ascii_digit() {
        start -= 1;
    }
    (start < end).then_some(&line[start..end])
}

fn first_token(input: &[u8]) -> &[u8] {
    let input = input.trim_ascii();
    let end = input
        .iter()
        .position(|byte| matches!(byte, b' ' | b'\t'))
        .unwrap_or(input.len());
    &input[..end]
}

fn write_line(output: &mut Vec<u8>, line: &[u8]) {
    output.extend_from_slice(line);
    output.push(b'\n');
}

fn head_tail(input: Vec<u8>, head: usize, tail: usize) -> Vec<u8> {
    let lines: Vec<&[u8]> = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() <= head + tail {
        return input;
    }
    let omitted = lines.len() - head - tail;
    let mut output = Vec::with_capacity(input.len());
    for line in &lines[..head] {
        write_line(&mut output, line);
    }
    output.extend_from_slice(b"(smll: omitted ");
    output.extend_from_slice(omitted.to_string().as_bytes());
    output.extend_from_slice(b" relevant lines; rerun with smll --raw)\n");
    for line in &lines[lines.len() - tail..] {
        write_line(&mut output, line);
    }
    output
}
