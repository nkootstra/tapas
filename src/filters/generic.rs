use std::collections::{HashMap, HashSet};

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
    let mut cleaned = Vec::<Vec<u8>>::new();
    for raw in input.split(|byte| *byte == b'\n') {
        let line = collapse_whitespace(trim_end(&strip_ansi(raw)));
        cleaned.push(line);
    }

    let mut frequencies: HashMap<&[u8], usize> = HashMap::new();
    for line in &cleaned {
        if !line.is_empty() {
            *frequencies.entry(line).or_default() += 1;
        }
    }

    let mut emitted = HashSet::<&[u8]>::new();
    let capacity = cleaned
        .iter()
        .map(|line| line.len().saturating_add(1))
        .sum::<usize>()
        .min(input.len());
    let mut output = Vec::with_capacity(capacity);
    let mut previous: Option<&[u8]> = None;
    let mut pending_blank = false;

    for line in &cleaned {
        if line.is_empty() {
            pending_blank = true;
            continue;
        }
        let body = line.as_slice();

        if previous == Some(body) {
            pending_blank = false;
            continue;
        }

        if let Some(previous_body) = previous {
            if pending_blank && !output.is_empty() {
                output.push(b'\n');
            }
            append_output_line(&mut output, previous_body, frequencies[previous_body]);
        }

        let frequency = frequencies[body];
        if frequency >= 3 && emitted.contains(body) {
            previous = None;
            pending_blank = false;
            continue;
        }

        pending_blank = false;
        if frequency >= 3 {
            emitted.insert(body);
        }
        previous = Some(body);
    }

    if let Some(previous_body) = previous {
        append_output_line(&mut output, previous_body, frequencies[previous_body]);
    }

    Ok(FilterOutput::new(output, EvidenceClass::FactComplete))
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
