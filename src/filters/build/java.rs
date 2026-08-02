use super::*;

#[derive(Default)]
struct GradleState {
    in_cause_block: bool,
    after_failure_context: usize,
}

pub(super) fn compact_gradle(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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

pub(super) fn matches_maven(input: &[u8]) -> bool {
    [
        b"[ERROR]".as_slice(),
        b"[WARNING]",
        b"BUILD FAILURE",
        b"BUILD SUCCESS",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

pub(super) fn compact_maven(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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
