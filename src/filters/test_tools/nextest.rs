use super::append_line;

pub(super) fn route(argv: &[&[u8]]) -> bool {
    matches!(
        argv,
        [b"nextest", b"run", ..] | [b"cargo", b"nextest", b"run", ..]
    )
}

pub(super) fn compact(input: &[u8]) -> Option<Vec<u8>> {
    let lines: Vec<&[u8]> = input.split(|byte| *byte == b'\n').collect();
    let has_start = lines.iter().any(|line| {
        let line = line.trim_ascii();
        line.starts_with(b"Starting ")
            && line
                .windows(b" tests across ".len())
                .any(|part| part == b" tests across ")
    });
    let has_summary = lines
        .iter()
        .any(|line| line.trim_ascii().starts_with(b"Summary [") && line.contains(&b':'));
    if !has_start || !has_summary {
        return None;
    }

    let first_failure = lines
        .iter()
        .position(|line| line.trim_ascii().starts_with(b"FAIL ["));
    let mut output = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        let classified = line.trim_ascii();
        if classified.is_empty() {
            continue;
        }
        if classified.starts_with(b"Summary [")
            || first_failure.is_some_and(|failure| index >= failure)
                && !classified.starts_with(b"PASS [")
        {
            append_line(&mut output, line.trim_ascii_end());
        }
    }
    Some(output)
}
