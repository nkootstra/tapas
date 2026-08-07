use super::super::{append_line, find_subslice, strip_ansi_csi as strip_ansi};

pub(super) fn matches_plan(input: &[u8]) -> bool {
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

pub(super) fn compact_plan(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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
