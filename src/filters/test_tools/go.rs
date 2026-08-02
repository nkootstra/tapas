use super::*;

pub(super) fn matches_go_test(input: &[u8]) -> bool {
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

pub(super) fn apply_go_test(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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
            append_line(output, line);
            *has_benchmark_or_fuzz = true;
            continue;
        }
        if line.starts_with(b"fuzz: ") {
            last_fuzz_progress.clear();
            append_line(&mut last_fuzz_progress, line);
            *has_benchmark_or_fuzz = true;
            continue;
        }
        if line.starts_with(b"--- FUZZ:") || line.starts_with(b"=== FUZZ") {
            output.append(&mut pending);
            append_line(output, line);
            *has_benchmark_or_fuzz = true;
            continue;
        }
        if line.starts_with(b"--- FAIL:") {
            output.append(&mut pending);
            append_line(output, line);
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
            append_line(&mut pending, line);
            continue;
        }
        output.append(&mut pending);
        if line.starts_with(b"FAIL\t") || line.starts_with(b"ok\t") || line.starts_with(b"ok  ") {
            append_line(output, line);
        }
    }
    output.extend_from_slice(&last_fuzz_progress);
}

fn is_go_benchmark(line: &[u8]) -> bool {
    line.starts_with(b"Benchmark") && line.contains(&b'\t')
}
