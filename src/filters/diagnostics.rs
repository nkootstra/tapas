use super::{
    EvidenceClass, FilterError, StreamFilterDecision, StreamFilterOutput, append_line,
    command_basename, contains_ignore_ascii_case as contains_ascii_case_insensitive, find_subslice,
    strip_ansi_csi as strip_ansi,
};

pub(crate) fn handles_argv(argv: &[&[u8]]) -> bool {
    argv.first()
        .copied()
        .map(command_basename)
        .is_some_and(|command| {
            matches!(
                command,
                b"mypy"
                    | b"ruff"
                    | b"eslint"
                    | b"biome"
                    | b"pre-commit"
                    | b"prettier"
                    | b"terraform"
                    | b"tofu"
            )
        })
}

pub fn dispatch_streams_argv(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    _exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterOutput, FilterError> {
    dispatch_streams_decision(argv, stdout, stderr, _exit_code, lossless)
        .map(|decision| decision.into_output(stdout, stderr))
}

pub(crate) fn dispatch_streams_decision(
    argv: &[&[u8]],
    stdout: &[u8],
    stderr: &[u8],
    _exit_code: i32,
    lossless: bool,
) -> Result<StreamFilterDecision, FilterError> {
    let Some(command) = argv.first().copied().map(command_basename) else {
        return Err(FilterError::InvalidInput);
    };
    if lossless || crate::invocation_policy::requests_passthrough(argv) {
        return Ok(StreamFilterDecision::Unchanged);
    }

    type Compact = fn(&[u8], &[u8]) -> Vec<u8>;
    let output: Option<(Compact, EvidenceClass)> = match command {
        b"mypy" => Some((compact_mypy, EvidenceClass::FactComplete)),
        b"ruff" => Some((compact_ruff, EvidenceClass::FactComplete)),
        b"eslint" | b"biome" if matches_lint(stdout) || matches_lint(stderr) => {
            Some((compact_lint, EvidenceClass::FactComplete))
        }
        b"pre-commit" => Some((compact_precommit, EvidenceClass::FactComplete)),
        b"prettier" => Some((compact_prettier, EvidenceClass::FactComplete)),
        b"terraform" | b"tofu"
            if argv.get(1).copied() == Some(b"plan")
                && (matches_plan(stdout) || matches_plan(stderr)) =>
        {
            Some((compact_plan, EvidenceClass::FactComplete))
        }
        _ => None,
    };

    Ok(output.map_or_else(
        || StreamFilterDecision::Unchanged,
        |(compact, evidence)| {
            StreamFilterDecision::compact_single_stream(stdout, stderr, evidence, compact)
        },
    ))
}

fn compact_mypy(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_mypy(stdout, &mut output);
    scan_mypy(stderr, &mut output);
    output
}

fn scan_mypy(input: &[u8], output: &mut Vec<u8>) {
    let mut in_diagnostic = false;
    for raw in input.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            in_diagnostic = false;
            continue;
        }
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if is_mypy_diagnostic(line)
            || line.starts_with(b"Found ")
            || line.starts_with(b"Success: ")
            || line.starts_with(b"mypy: ")
        {
            append_line(output, line);
            in_diagnostic = is_mypy_diagnostic(line);
        } else if in_diagnostic && is_caret_line(line) {
            append_line(output, line);
            in_diagnostic = false;
        }
    }
}

fn is_mypy_diagnostic(line: &[u8]) -> bool {
    find_subslice(line, b": error:").is_some() || find_subslice(line, b": note:").is_some()
}

fn is_caret_line(line: &[u8]) -> bool {
    let line = line.trim_ascii();
    !line.is_empty() && line.iter().all(|byte| matches!(byte, b'^' | b'~'))
}

fn compact_ruff(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut current_path = Vec::new();
    scan_ruff(stdout, &mut output, &mut current_path);
    scan_ruff(stderr, &mut output, &mut current_path);
    output
}

fn scan_ruff(input: &[u8], output: &mut Vec<u8>, current_path: &mut Vec<u8>) {
    for raw in input.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if let Some((path, location, body)) = parse_ruff_diagnostic(line) {
            if current_path != path {
                append_line(output, path);
                current_path.clear();
                current_path.extend_from_slice(path);
            }
            output.extend_from_slice(b"  ");
            output.extend_from_slice(location);
            output.push(b' ');
            append_line(output, body);
        } else if is_ruff_summary(line) {
            append_line(output, line);
            current_path.clear();
        }
    }
}

fn parse_ruff_diagnostic(line: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    let first = line.iter().position(|byte| *byte == b':')?;
    let mut cursor = first + 1;
    let line_start = cursor;
    while line.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == line_start || line.get(cursor) != Some(&b':') {
        return None;
    }
    cursor += 1;
    let column_start = cursor;
    while line.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == column_start || line.get(cursor) != Some(&b':') {
        return None;
    }
    Some((
        &line[..first],
        &line[first + 1..cursor],
        line[cursor + 1..].trim_ascii_start(),
    ))
}

fn is_ruff_summary(line: &[u8]) -> bool {
    line.starts_with(b"All checks passed")
        || line.starts_with(b"Found ")
        || line.ends_with(b"would be reformatted")
        || line.ends_with(b"left unchanged")
        || find_subslice(line, b" files would be reformatted").is_some()
        || find_subslice(line, b" files left unchanged").is_some()
}

fn matches_lint(input: &[u8]) -> bool {
    find_subslice(input, b" problems").is_some()
        || find_subslice(input, b" problem").is_some()
        || find_subslice(input, b"lint/").is_some()
        || find_subslice(input, b"error").is_some() && find_subslice(input, b"warning").is_some()
}

fn compact_lint(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut state = LintState::default();
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_lint(stdout, &mut output, &mut state);
    scan_lint(stderr, &mut output, &mut state);
    if !state.emitted && (!stdout.is_empty() || !stderr.is_empty()) {
        output.extend_from_slice(b"lint ok\n");
    }
    output
}

#[derive(Default)]
struct LintState {
    pending_file: Vec<u8>,
    pending_emitted: bool,
    emitted: bool,
}

fn scan_lint(input: &[u8], output: &mut Vec<u8>, state: &mut LintState) {
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if looks_like_file_header(line) {
            state.pending_file.clear();
            state.pending_file.extend_from_slice(line);
            state.pending_emitted = false;
        } else if looks_like_lint_diagnostic(line) {
            if !state.pending_file.is_empty() && !state.pending_emitted {
                append_line(output, &state.pending_file);
                state.pending_emitted = true;
            }
            append_collapsed(output, line);
            state.emitted = true;
        } else if looks_like_lint_summary(line) {
            append_line(output, line);
            state.emitted = true;
        }
    }
}

fn looks_like_file_header(line: &[u8]) -> bool {
    !line.is_empty()
        && !line.contains(&b' ')
        && (line.contains(&b'/')
            || [
                b".js".as_slice(),
                b".jsx",
                b".ts",
                b".tsx",
                b".vue",
                b".svelte",
            ]
            .iter()
            .any(|suffix| line.ends_with(suffix)))
}

fn looks_like_lint_diagnostic(line: &[u8]) -> bool {
    if ![b"error".as_slice(), b"warning", b"lint/"]
        .iter()
        .any(|needle| find_subslice(line, needle).is_some())
    {
        return false;
    }
    if numeric_location_prefix(line) {
        return true;
    }
    let Some(first) = line.iter().position(|byte| *byte == b':') else {
        return false;
    };
    numeric_location_prefix(&line[first + 1..])
}

fn numeric_location_prefix(line: &[u8]) -> bool {
    let mut cursor = 0;
    while line.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == 0 || line.get(cursor) != Some(&b':') {
        return false;
    }
    cursor += 1;
    let start = cursor;
    while line.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    cursor > start && cursor < line.len()
}

fn looks_like_lint_summary(line: &[u8]) -> bool {
    find_subslice(line, b" problems").is_some()
        || find_subslice(line, b" problem").is_some()
        || line.starts_with(b"Found ")
        || line.starts_with(b"Checked ")
        || line.starts_with(b"No lint errors")
}

fn compact_precommit(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut passed = 0usize;
    scan_precommit(stdout, &mut output, &mut passed);
    scan_precommit(stderr, &mut output, &mut passed);
    if passed > 0 {
        output.extend_from_slice(b"passed: ");
        output.extend_from_slice(passed.to_string().as_bytes());
        output.extend_from_slice(if passed == 1 { b" hook\n" } else { b" hooks\n" });
    }
    output
}

fn scan_precommit(input: &[u8], output: &mut Vec<u8>, passed: &mut usize) {
    let mut in_failure = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if let Some(status) = hook_status(line) {
            in_failure = status == b"Failed";
            if status == b"Passed" {
                *passed += 1;
            } else if status != b"Skipped" {
                append_depadded_status(output, line, status);
            }
        } else if in_failure && should_keep_failure_line(line) {
            append_line(output, line);
        }
    }
}

fn hook_status(line: &[u8]) -> Option<&'static [u8]> {
    [b"Passed".as_slice(), b"Failed", b"Skipped"]
        .into_iter()
        .find(|status| line_ends_with_dot_status(line, status))
}

fn line_ends_with_dot_status(line: &[u8], status: &[u8]) -> bool {
    let Some(prefix) = line.strip_suffix(status) else {
        return false;
    };
    prefix
        .iter()
        .rev()
        .take_while(|byte| **byte == b'.')
        .count()
        >= 3
}

fn append_depadded_status(output: &mut Vec<u8>, line: &[u8], status: &[u8]) {
    let mut end = line.len() - status.len();
    while end > 0 && line[end - 1] == b'.' {
        end -= 1;
    }
    output.extend_from_slice(line[..end].trim_ascii_end());
    output.push(b' ');
    append_line(output, status);
}

fn should_keep_failure_line(line: &[u8]) -> bool {
    line.starts_with(b"- hook id:")
        || line.starts_with(b"- exit code:")
        || contains_ascii_case_insensitive(line, b"error")
        || contains_ascii_case_insensitive(line, b"failed")
        || line.contains(&b':')
            && [b".py".as_slice(), b".yaml", b".yml", b".toml", b".json"]
                .iter()
                .any(|extension| find_subslice(line, extension).is_some())
}

fn compact_prettier(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut formatted = Vec::<Vec<u8>>::new();
    let mut total = 0usize;
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_prettier(stdout, &mut output, &mut formatted, &mut total);
    scan_prettier(stderr, &mut output, &mut formatted, &mut total);
    if total > 0 {
        output.extend_from_slice(b"formatted ");
        output.extend_from_slice(total.to_string().as_bytes());
        output.extend_from_slice(b": ");
        for (index, path) in formatted.iter().enumerate() {
            if index > 0 {
                output.extend_from_slice(b", ");
            }
            output.extend_from_slice(path);
        }
        if total > formatted.len() {
            output.extend_from_slice(b", ... (+");
            output.extend_from_slice((total - formatted.len()).to_string().as_bytes());
            output.push(b')');
        }
        output.push(b'\n');
    }
    output
}

fn scan_prettier(
    input: &[u8],
    output: &mut Vec<u8>,
    formatted: &mut Vec<Vec<u8>>,
    total: &mut usize,
) {
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if line.is_empty() {
            continue;
        }
        if let Some((path, changed)) = parse_prettier_write(line) {
            if changed {
                *total += 1;
                if formatted.len() < 8 {
                    formatted.push(path.to_vec());
                }
            }
        } else if line.starts_with(b"[warn]")
            || line.starts_with(b"[error]")
            || line.starts_with(b"All matched files use Prettier")
            || find_subslice(line, b"Code style issues found").is_some()
            || find_subslice(line, b"No files matching").is_some()
        {
            append_line(output, line);
        }
    }
}

fn parse_prettier_write(mut line: &[u8]) -> Option<(&[u8], bool)> {
    if line.starts_with(b"[warn]") || line.starts_with(b"[error]") {
        return None;
    }
    let changed = if let Some(prefix) = line.strip_suffix(b" (unchanged)") {
        line = prefix;
        false
    } else {
        true
    };
    let before_ms = line.strip_suffix(b"ms")?;
    let space = before_ms.iter().rposition(|byte| *byte == b' ')?;
    let duration = &before_ms[space + 1..];
    (!duration.is_empty() && duration.iter().all(u8::is_ascii_digit) && space > 0)
        .then_some((&before_ms[..space], changed))
}

fn matches_plan(input: &[u8]) -> bool {
    [
        b"Terraform will perform".as_slice(),
        b"OpenTofu will perform",
        b"Plan: ",
        b"No changes.",
        b"Error: ",
        b"Warning: ",
    ]
    .iter()
    .any(|needle| find_subslice(input, needle).is_some())
}

fn compact_plan(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    let mut emitted = false;
    scan_plan(stdout, &mut output, &mut emitted);
    scan_plan(stderr, &mut output, &mut emitted);
    if !emitted && (!stdout.is_empty() || !stderr.is_empty()) {
        output.extend_from_slice(b"plan ok\n");
    }
    output
}

fn scan_plan(input: &[u8], output: &mut Vec<u8>, emitted: &mut bool) {
    let mut in_actions = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if find_subslice(line, b"will perform the following actions").is_some() {
            in_actions = true;
        }
        if keep_plan_line(line, in_actions) {
            append_line(output, line);
            *emitted = true;
        }
    }
}

fn keep_plan_line(line: &[u8], in_actions: bool) -> bool {
    let diagnostic = line.strip_prefix("│ ".as_bytes()).unwrap_or(line);
    line.starts_with(b"Plan: ")
        || line.starts_with(b"No changes.")
        || diagnostic.starts_with(b"Error: ")
        || diagnostic.starts_with(b"Warning: ")
        || line.starts_with(b"Terraform will perform")
        || line.starts_with(b"OpenTofu will perform")
        || line.starts_with(b"# ")
            && (find_subslice(line, b" will be ").is_some()
                || find_subslice(line, b" must be ").is_some())
        || [
            b"+ resource ".as_slice(),
            b"- resource ",
            b"~ resource ",
            b"-/+ resource ",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        || in_actions
            && (find_subslice(line, b"# forces replacement").is_some()
                || [b"-/+ ".as_slice(), b"~ ", b"+ ", b"- "]
                    .iter()
                    .any(|prefix| line.starts_with(prefix)))
}

fn append_collapsed(output: &mut Vec<u8>, line: &[u8]) {
    let mut previous_space = false;
    for byte in line {
        if *byte == b' ' {
            if !previous_space {
                output.push(b' ');
            }
            previous_space = true;
        } else {
            output.push(*byte);
            previous_space = false;
        }
    }
    output.push(b'\n');
}
