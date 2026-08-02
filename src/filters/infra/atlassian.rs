pub(super) fn compact_acli(
    arg1: &[u8],
    arg2: &[u8],
    arg3: &[u8],
    stdout: &[u8],
) -> Option<Vec<u8>> {
    let table = arg1 == b"jira" && arg2 == b"workitem" && arg3 == b"search"
        || arg1 == b"confluence" && arg2 == b"space" && arg3 == b"list";
    if table {
        return Some(collapse_table(stdout));
    }
    let view = arg1 == b"jira" && arg2 == b"workitem" && arg3 == b"view"
        || arg1 == b"confluence" && arg2 == b"page" && arg3 == b"view";
    view.then(|| compact_acli_view(stdout))
}

fn compact_acli_view(stdout: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(stdout.len());
    let mut kept = 0usize;
    let mut pending_body = false;
    let mut pending_field = false;
    let mut in_fields = false;
    for raw in stdout.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if pending_body {
            pending_body = false;
            if !is_body_boundary(line) {
                output.extend_from_slice(b"  ");
                append_line(&mut output, line);
                kept += 1;
                continue;
            }
        }
        if pending_field {
            pending_field = false;
            if !looks_like_label(line) {
                output.extend_from_slice(b"  ");
                append_line(&mut output, line);
                kept += 1;
                continue;
            }
        }
        if line.starts_with(b"Fields:") {
            in_fields = true;
            append_line(&mut output, line);
            kept += 1;
        } else if is_body_label(line) {
            in_fields = false;
            append_line(&mut output, line);
            kept += 1;
            pending_body = line.ends_with(b":");
        } else if in_fields && looks_like_label(line) {
            append_line(&mut output, line);
            kept += 1;
            pending_field = line.ends_with(b":");
        } else if is_acli_metadata(line) {
            append_line(&mut output, line);
            kept += 1;
        }
    }
    if kept == 0 { stdout.to_vec() } else { output }
}

fn is_acli_metadata(line: &[u8]) -> bool {
    [
        b"Key:".as_slice(),
        b"Work item:",
        b"Issue:",
        b"Type:",
        b"Summary:",
        b"Status:",
        b"Assignee:",
        b"Priority:",
        b"Reporter:",
        b"Created:",
        b"Updated:",
        b"URL:",
        b"Web URL:",
        b"ID:",
        b"Title:",
        b"Space:",
        b"Author:",
        b"Created by:",
        b"Last updated:",
        b"Version:",
        b"Labels:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn is_body_label(line: &[u8]) -> bool {
    [b"Description:".as_slice(), b"Body:", b"Comments:"]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

fn is_body_boundary(line: &[u8]) -> bool {
    is_body_label(line) || line.starts_with(b"Fields:")
}

fn looks_like_label(line: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|byte| *byte == b':') else {
        return false;
    };
    colon > 0
        && colon <= 48
        && line[..colon].iter().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b' ' | b'-' | b'_' | b'/' | b'&' | b'(' | b')')
        })
}
use super::table::collapse_table;
use super::{append_line, strip_ansi};
