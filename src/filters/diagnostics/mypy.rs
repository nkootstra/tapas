use super::{RecognizedStream, split_location};
use crate::filters::{append_line, find_subslice, strip_ansi_csi};

pub(super) fn classify_mypy(input: &[u8]) -> Option<RecognizedStream> {
    if input.is_empty() {
        return None;
    }

    let mut output = Vec::with_capacity(input.len());
    let mut found_diagnostic = false;
    let mut found_clean_summary = false;
    let mut in_diagnostic = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi_csi(raw);
        let line = clean.trim_ascii_end();
        if is_mypy_diagnostic(line) {
            append_line(&mut output, line);
            found_diagnostic = true;
            in_diagnostic = true;
        } else if in_diagnostic
            && !line.is_empty()
            && clean.first().is_some_and(u8::is_ascii_whitespace)
        {
            append_line(&mut output, line);
        } else if is_summary(line) {
            append_line(&mut output, line);
            found_clean_summary |= line.starts_with(b"Success: ");
            in_diagnostic = false;
        } else if !line.is_empty() {
            in_diagnostic = false;
        }
    }

    if found_diagnostic {
        Some(RecognizedStream::Diagnostics(output))
    } else if found_clean_summary {
        Some(RecognizedStream::Clean(output))
    } else {
        None
    }
}

fn is_mypy_diagnostic(line: &[u8]) -> bool {
    if line.starts_with(b"mypy: error:") {
        return true;
    }
    let Some((_, _, severity)) = split_location(line, false) else {
        return false;
    };
    [b"error:".as_slice(), b"note:", b"warning:"]
        .iter()
        .any(|prefix| severity.trim_ascii_start().starts_with(prefix))
}

fn is_summary(line: &[u8]) -> bool {
    line.starts_with(b"Found ")
        && (find_subslice(line, b" error").is_some() || find_subslice(line, b" errors").is_some())
        || line.starts_with(b"Success: ")
}
