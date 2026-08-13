use super::find_subslice;

pub(super) fn compact(input: &[u8]) -> Option<Vec<u8>> {
    if let Some(start) = failure_start(input)
        && (find_subslice(&input[start..], b"Error Message:").is_some()
            || find_subslice(&input[start..], b"Stack Trace:").is_some())
    {
        return Some(input[start..].to_vec());
    }

    let summary = summary_start(input)?;
    if find_subslice(input, b"Test run for ").is_some()
        && find_subslice(input, b"Starting test execution").is_some()
    {
        return Some(input[summary..].to_vec());
    }

    if summary == 0 || input[..summary].iter().all(u8::is_ascii_whitespace) {
        return Some(input[summary..].to_vec());
    }
    None
}

fn failure_start(input: &[u8]) -> Option<usize> {
    input
        .split_inclusive(|byte| *byte == b'\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line.trim_ascii()))
        })
        .find_map(|(offset, line)| {
            (line.starts_with(b"[xUnit.net ") && line.ends_with(b"[FAIL]")
                || line.starts_with(b"Failed "))
            .then_some(offset)
        })
}

fn summary_start(input: &[u8]) -> Option<usize> {
    input
        .split_inclusive(|byte| *byte == b'\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line.trim_ascii()))
        })
        .find_map(|(offset, line)| recognized_summary(line).then_some(offset))
}

fn recognized_summary(line: &[u8]) -> bool {
    let Some(rest) = line
        .strip_prefix(b"Passed!  - ")
        .or_else(|| line.strip_prefix(b"Failed!  - "))
    else {
        return false;
    };
    let Some(rest) = numeric_field(rest, b"Failed: ") else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(b", Passed: ") else {
        return false;
    };
    let Some(rest) = digits_then(rest) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(b", Skipped: ") else {
        return false;
    };
    let Some(rest) = digits_then(rest) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(b", Total: ") else {
        return false;
    };
    digits_then(rest).is_some_and(|suffix| suffix.is_empty() || suffix.starts_with(b", Duration: "))
}

fn numeric_field<'a>(input: &'a [u8], label: &[u8]) -> Option<&'a [u8]> {
    digits_then(input.strip_prefix(label)?)
}

fn digits_then(input: &[u8]) -> Option<&[u8]> {
    let digits = input
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    (digits > 0).then_some(&input[digits..])
}
