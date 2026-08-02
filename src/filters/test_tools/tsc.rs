#[derive(Debug)]
struct TscDiagnostic {
    location: Vec<u8>,
    rest: Vec<u8>,
    code: Vec<u8>,
    message: Vec<u8>,
}

pub(super) fn matches_tsc(input: &[u8]) -> bool {
    find_subslice(input, b"error TS").is_some()
        || find_subslice(input, b"Found 0 errors").is_some()
        || (find_subslice(input, b"Found ").is_some()
            && find_subslice(input, b" errors in ").is_some())
}

pub(super) fn apply_tsc(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
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
        append_line(&mut output, line);
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
                append_line(output, &diagnostics[index].message);
            }
        } else {
            for candidate in group {
                output.extend_from_slice(&diagnostics[candidate].location);
                output.push(b' ');
                append_line(output, &diagnostics[candidate].rest);
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
use super::{append_line, find_subslice, strip_ansi};
