use super::{RecognizedStream, split_location};
use crate::filters::{append_line, strip_ansi_csi};

pub(super) fn classify_golangci_lint(input: &[u8]) -> Option<RecognizedStream> {
    if input.is_empty() {
        return None;
    }

    let mut output = Vec::with_capacity(input.len());
    let mut found_diagnostic = false;
    let mut in_diagnostic = false;
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi_csi(raw);
        let line = clean.trim_ascii_end();
        if is_diagnostic(line) {
            append_line(&mut output, line);
            found_diagnostic = true;
            in_diagnostic = true;
        } else if in_diagnostic && clean.first().is_some_and(u8::is_ascii_whitespace) {
            append_line(&mut output, line);
        } else if is_summary(line) {
            append_line(&mut output, line);
            in_diagnostic = false;
        } else if !line.is_empty() {
            in_diagnostic = false;
        }
    }

    found_diagnostic.then_some(RecognizedStream::Diagnostics(output))
}

fn is_diagnostic(line: &[u8]) -> bool {
    let Some((_, _, body)) = split_location(line, false) else {
        return false;
    };
    let body = body.trim_ascii_start();
    body.iter()
        .rposition(|byte| *byte == b'(')
        .map(|open| &body[open + 1..])
        .is_some_and(|linter| {
            linter.ends_with(b")")
                && linter.len() > 1
                && linter[..linter.len() - 1]
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn is_summary(line: &[u8]) -> bool {
    line.strip_suffix(b" issues:")
        .is_some_and(|count| !count.is_empty() && count.iter().all(u8::is_ascii_digit))
}
