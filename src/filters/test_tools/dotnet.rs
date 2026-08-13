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
    let status = line.starts_with(b"Passed!") || line.starts_with(b"Failed!");
    status
        && find_subslice(line, b"Failed:").is_some()
        && find_subslice(line, b"Passed:").is_some()
        && find_subslice(line, b"Total:").is_some()
}
