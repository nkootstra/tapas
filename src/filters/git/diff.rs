pub(super) fn apply_diff(input: &[u8]) -> Vec<u8> {
    let had_trailing_newline = input.ends_with(b"\n");
    let content = input.strip_suffix(b"\n").unwrap_or(input);
    let mut output = Vec::with_capacity(input.len());
    let mut first = true;

    for raw in content.split(|byte| *byte == b'\n') {
        let stripped = strip_ansi(raw);
        let line = stripped.as_slice();
        let transformed = if line.starts_with(b"diff --git a/") {
            let rest = &line[b"diff --git a/".len()..];
            let mut value = b"d ".to_vec();
            if let Some(split) = rfind_subslice(rest, b" b/") {
                let old = &rest[..split];
                let new = &rest[split + 3..];
                value.extend_from_slice(old);
                if old != new {
                    value.extend_from_slice(b" -> ");
                    value.extend_from_slice(new);
                }
            } else {
                value.extend_from_slice(rest);
            }
            Some(value)
        } else if line.starts_with(b"@@ ") {
            Some(compact_hunk_header(line))
        } else if is_diff_metadata(line) {
            None
        } else if line.starts_with(b"Binary files ") && line.ends_with(b" differ") {
            Some(b"B".to_vec())
        } else {
            Some(line.to_vec())
        };

        if let Some(line) = transformed {
            if !first {
                output.push(b'\n');
            }
            output.extend_from_slice(&line);
            first = false;
        }
    }

    if had_trailing_newline && !first {
        output.push(b'\n');
    }
    output
}

fn compact_hunk_header(line: &[u8]) -> Vec<u8> {
    let after_open = &line[3..];
    let Some(close) = find_subslice(after_open, b" @@") else {
        let mut output = b"@ ".to_vec();
        output.extend_from_slice(after_open);
        return output;
    };
    let coords = &after_open[..close];
    let context = &after_open[close + 3..];
    let mut output = b"@".to_vec();
    if let Some(split) = find_subslice(coords, b" +") {
        output.extend_from_slice(
            coords[..split]
                .strip_prefix(b"-")
                .unwrap_or(&coords[..split]),
        );
        output.push(b'|');
        output.extend_from_slice(&coords[split + 2..]);
    } else {
        output.extend_from_slice(coords);
    }
    output.extend_from_slice(context);
    output
}

fn is_diff_metadata(line: &[u8]) -> bool {
    [
        b"index ".as_slice(),
        b"similarity index ",
        b"dissimilarity index ",
        b"--- a/",
        b"--- /dev/null",
        b"+++ b/",
        b"+++ /dev/null",
        b"rename from ",
        b"rename to ",
        b"copy from ",
        b"copy to ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

pub(super) fn first_nonempty_line(input: &[u8]) -> Option<&[u8]> {
    input
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
}
use super::{find_subslice, rfind_subslice, strip_ansi};
