use super::{EvidenceClass, FilterError, FilterOutput, strip_ansi};

pub const THRESHOLD_BYTES: usize = 4 * 1024;

pub fn always(_: crate::signals::Signals) -> bool {
    true
}

pub fn matches(input: &[u8]) -> bool {
    if input.len() <= THRESHOLD_BYTES || looks_like_json(input) {
        return false;
    }

    let sample = &input[..input.len().min(512)];
    let mut controls = 0;
    for &byte in sample {
        if byte == 0 {
            return false;
        }
        if byte < 0x20 && !matches!(byte, b'\n' | b'\r' | b'\t' | 0x1b) {
            controls += 1;
        }
    }
    controls * 10 <= sample.len()
}

pub(crate) fn apply_matched(input: &[u8]) -> Result<FilterOutput, FilterError> {
    // Store cleaned lines in one buffer with offsets instead of one Vec per
    // line: a newline-heavy input would otherwise allocate a descriptor per
    // (often empty) line. The trailing offset marks the end of the last line.
    let mut lines = Vec::new();
    let mut offsets = Vec::new();
    let mut start = 0;
    for raw in input.split(|byte| *byte == b'\n') {
        lines.extend_from_slice(&collapse_whitespace(trim_end(&strip_ansi(raw))));
        offsets.push(start);
        start = lines.len();
    }
    offsets.push(start);
    let line = |index: usize| &lines[offsets[index]..offsets[index + 1]];
    // A trailing newline terminates the last line rather than starting an
    // empty one.
    let line_count = offsets.len() - 1 - usize::from(input.ends_with(b"\n"));

    // Only consecutive runs of an identical cleaned line collapse, to one line
    // with its run length. Lines that repeat elsewhere in the stream keep every
    // occurrence because their position carries meaning (code fences, closing
    // braces, patch lines). A run of blank lines keeps one blank line.
    let mut output = Vec::with_capacity(lines.len().min(input.len()));
    let mut removed_lines = false;
    let mut index = 0;
    while index < line_count {
        let body = line(index);
        let mut end = index + 1;
        while end < line_count && line(end) == body {
            end += 1;
        }
        let run = end - index;
        removed_lines |= run > 1;
        let count = if body.is_empty() { 1 } else { run };
        append_output_line(&mut output, body, count);
        index = end;
    }

    let evidence = if removed_lines {
        EvidenceClass::PotentiallyLossy
    } else {
        EvidenceClass::FactComplete
    };
    Ok(FilterOutput::new(output, evidence))
}

fn looks_like_json(input: &[u8]) -> bool {
    let mut trimmed = trim_start(input);
    if trimmed.starts_with(&[0xef, 0xbb, 0xbf]) {
        trimmed = &trimmed[3..];
    }
    let Some((&opening, rest)) = trimmed.split_first() else {
        return false;
    };
    match opening {
        b'{' => matches!(trim_start(rest).first(), Some(b'"' | b'}')),
        b'[' => looks_like_json_array(trim_start(rest)),
        _ => false,
    }
}

fn looks_like_json_array(input: &[u8]) -> bool {
    let Some(&first) = input.first() else {
        return false;
    };
    if matches!(first, b']' | b'{' | b'[' | b'"') {
        return true;
    }
    for literal in [b"true".as_slice(), b"false", b"null"] {
        if let Some(rest) = input.strip_prefix(literal) {
            return json_value_has_delimiter(rest);
        }
    }
    matches!(first, b'-' | b'0'..=b'9') && json_number_has_delimiter(input)
}

fn json_number_has_delimiter(input: &[u8]) -> bool {
    let mut index = usize::from(input.first() == Some(&b'-'));
    let digits_start = index;
    while matches!(input.get(index), Some(b'0'..=b'9')) {
        index += 1;
    }
    if index == digits_start {
        return false;
    }
    if input.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while matches!(input.get(index), Some(b'0'..=b'9')) {
            index += 1;
        }
        if index == fraction_start {
            return false;
        }
    }
    if matches!(input.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(input.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while matches!(input.get(index), Some(b'0'..=b'9')) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    json_value_has_delimiter(&input[index..])
}

fn json_value_has_delimiter(input: &[u8]) -> bool {
    matches!(trim_start(input).first(), Some(b',' | b']'))
}

fn trim_start(mut input: &[u8]) -> &[u8] {
    while input
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
    {
        input = &input[1..];
    }
    input
}

fn trim_end(mut input: &[u8]) -> &[u8] {
    while input
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[..input.len() - 1];
    }
    input
}

fn collapse_whitespace(input: &[u8]) -> Vec<u8> {
    let leading = input
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(input.len());
    let mut output = Vec::with_capacity(input.len());
    output.extend_from_slice(&input[..leading]);
    let mut index = leading;
    while index < input.len() {
        if matches!(input[index], b' ' | b'\t') {
            output.push(b' ');
            while index < input.len() && matches!(input[index], b' ' | b'\t') {
                index += 1;
            }
        } else {
            output.push(input[index]);
            index += 1;
        }
    }
    output
}

fn append_output_line(output: &mut Vec<u8>, line: &[u8], count: usize) {
    output.extend_from_slice(line);
    if count > 1 {
        output.extend_from_slice(" ×".as_bytes());
        output.extend_from_slice(count.to_string().as_bytes());
    }
    output.push(b'\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_repeated_patch_lines_in_place() {
        let mut patch = String::from(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n",
        );
        for hunk in 0..80 {
            patch.push_str(&format!("@@ -{0},3 +{0},7 @@\n", hunk * 10 + 1));
            patch.push_str(&format!(" fn existing_{hunk}() {{}}\n"));
            patch.push_str(&format!("+fn added_{hunk}() {{\n+work({hunk});\n+}}\n"));
            patch.push_str("+\n");
        }
        assert!(matches(patch.as_bytes()));

        let output = apply_matched(patch.as_bytes()).unwrap();
        let rendered = String::from_utf8(output.bytes).unwrap();
        assert_eq!(rendered.lines().filter(|line| *line == "+}").count(), 80);
        assert_eq!(rendered.lines().filter(|line| *line == "+").count(), 80);
        assert_eq!(rendered, patch);
        assert_eq!(output.evidence, EvidenceClass::FactComplete);
    }
}
