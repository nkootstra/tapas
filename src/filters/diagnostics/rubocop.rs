use super::{RecognizedStream, split_location};
use crate::filters::{append_line, find_subslice, strip_ansi_csi};

pub(super) fn classify_rubocop(input: &[u8]) -> Option<RecognizedStream> {
    if input.is_empty() {
        return None;
    }

    let mut output = Vec::with_capacity(input.len());
    let mut found_diagnostic = false;
    let mut found_summary = false;
    let mut in_diagnostic = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi_csi(raw);
        let line = clean.trim_ascii_end();
        if is_offense(line) {
            append_line(&mut output, line);
            found_diagnostic = true;
            in_diagnostic = true;
        } else if is_summary(line) {
            append_line(&mut output, line);
            found_summary = true;
            in_diagnostic = false;
        } else if in_diagnostic && !line.is_empty() {
            append_line(&mut output, line);
        } else if line.is_empty() {
            in_diagnostic = false;
        }
    }

    if found_diagnostic && found_summary {
        Some(RecognizedStream::Diagnostics(output))
    } else if found_summary && is_clean_summary(&output) {
        Some(RecognizedStream::Clean(output))
    } else {
        None
    }
}

fn is_offense(line: &[u8]) -> bool {
    let Some((_, _, rest)) = split_location(line, true) else {
        return false;
    };
    let rest = rest.trim_ascii_start();
    rest.get(1..3) == Some(b": ")
        && rest[0].is_ascii_uppercase()
        && find_subslice(rest, b"/").is_some()
        && find_subslice(rest, b": ").is_some()
}

fn is_summary(line: &[u8]) -> bool {
    find_subslice(line, b" inspected,").is_some()
        && (find_subslice(line, b" offense detected").is_some()
            || find_subslice(line, b" offenses detected").is_some()
            || find_subslice(line, b"no offenses detected").is_some())
}

fn is_clean_summary(output: &[u8]) -> bool {
    find_subslice(output, b"no offenses detected").is_some()
}
