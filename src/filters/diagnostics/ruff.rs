use std::borrow::Cow;

use super::{RecognizedStream, split_location};
use crate::filters::{append_line, find_subslice, strip_ansi_csi};

pub(super) fn classify_ruff(input: &[u8]) -> Option<RecognizedStream> {
    if input.is_empty() {
        return None;
    }

    let lines: Vec<Cow<'_, [u8]>> = input
        .split(|byte| *byte == b'\n')
        .map(|line| {
            if line.contains(&0x1b) {
                let mut clean = strip_ansi_csi(line);
                clean.truncate(clean.trim_ascii_end().len());
                Cow::Owned(clean)
            } else {
                Cow::Borrowed(line.trim_ascii_end())
            }
        })
        .collect();
    if has_full_diagnostic(&lines) {
        return Some(RecognizedStream::Diagnostics(compact_full(&lines)));
    }

    compact_concise(&lines)
}

fn has_full_diagnostic(lines: &[Cow<'_, [u8]>]) -> bool {
    lines.iter().enumerate().any(|(index, line)| {
        is_rule_header(line)
            && lines[index + 1..]
                .iter()
                .take_while(|candidate| !is_rule_header(candidate) && !is_summary(candidate))
                .any(|candidate| is_arrow_location(candidate))
    })
}

fn compact_full(lines: &[Cow<'_, [u8]>]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut in_diagnostic = false;
    for line in lines {
        if is_rule_header(line.as_ref()) {
            in_diagnostic = true;
            append_line(&mut output, line);
        } else if is_summary(line.as_ref()) {
            append_line(&mut output, line);
            in_diagnostic = false;
        } else if in_diagnostic {
            append_line(&mut output, line);
        }
    }
    output
}

fn compact_concise(lines: &[Cow<'_, [u8]>]) -> Option<RecognizedStream> {
    let mut output = Vec::new();
    let mut current_path = Vec::new();
    let mut found_diagnostic = false;
    let mut found_clean_summary = false;
    let mut in_diagnostic = false;
    for line in lines {
        let line = line.as_ref();
        if let Some((path, location, body)) = parse_concise_diagnostic(line) {
            if current_path != path {
                append_line(&mut output, path);
                current_path.clear();
                current_path.extend_from_slice(path);
            }
            output.extend_from_slice(b"  ");
            output.extend_from_slice(location);
            output.push(b' ');
            append_line(&mut output, body);
            found_diagnostic = true;
            in_diagnostic = true;
        } else if in_diagnostic && line.first().is_some_and(u8::is_ascii_whitespace) {
            append_line(&mut output, line);
        } else if is_summary(line) {
            append_line(&mut output, line);
            found_clean_summary |= line.starts_with(b"All checks passed");
            current_path.clear();
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

fn parse_concise_diagnostic(line: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    let (path, location, body) = split_location(line, true)?;
    let body = body.trim_ascii_start();
    let code = body.split(|byte| byte.is_ascii_whitespace()).next()?;
    if !is_rule_code(code) {
        return None;
    }
    Some((path, location, body))
}

fn is_rule_header(line: &[u8]) -> bool {
    let code = line
        .split(|byte| byte.is_ascii_whitespace())
        .next()
        .unwrap_or_default();
    is_rule_code(code) && line.get(code.len()).is_some_and(u8::is_ascii_whitespace)
}

fn is_rule_code(code: &[u8]) -> bool {
    let letters = code
        .iter()
        .take_while(|byte| byte.is_ascii_uppercase())
        .count();
    letters > 0 && letters < code.len() && code[letters..].iter().all(u8::is_ascii_digit)
}

fn is_arrow_location(line: &[u8]) -> bool {
    let line = line.trim_ascii_start();
    line.starts_with(b"--> ") && parse_location(line[4..].trim_ascii()).is_some()
}

fn parse_location(line: &[u8]) -> Option<()> {
    let last = line.iter().rposition(|byte| *byte == b':')?;
    if last == 0 || !line[last + 1..].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let previous = line[..last].iter().rposition(|byte| *byte == b':')?;
    (previous > 0 && line[previous + 1..last].iter().all(u8::is_ascii_digit)).then_some(())
}

fn is_summary(line: &[u8]) -> bool {
    line.starts_with(b"All checks passed")
        || line.starts_with(b"Found ")
        || line.ends_with(b"would be reformatted")
        || line.ends_with(b"left unchanged")
        || find_subslice(line, b" files would be reformatted").is_some()
        || find_subslice(line, b" files left unchanged").is_some()
}
