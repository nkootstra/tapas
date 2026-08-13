use super::find_subslice;

pub(super) fn human_route(argv: &[&[u8]]) -> bool {
    let mut format = None;
    let mut index = 1;
    while index < argv.len() {
        let argument = argv[index];
        if argument == b"--" {
            break;
        }
        if matches!(argument, b"--format" | b"-f") {
            let Some(value) = argv.get(index + 1).copied() else {
                return false;
            };
            if format.replace(value).is_some() {
                return false;
            }
            index += 2;
            continue;
        }
        if matches!(argument, b"--out" | b"-o")
            || argument.starts_with(b"--out=")
            || argument.starts_with(b"-o") && argument.len() > 2
        {
            return false;
        }
        if let Some(value) = argument.strip_prefix(b"--format=") {
            if format.replace(value).is_some() {
                return false;
            }
        } else if argument.starts_with(b"-f")
            && argument.len() > 2
            && format.replace(&argument[2..]).is_some()
        {
            return false;
        }
        index += 1;
    }
    format.is_none_or(|value| matches!(value, b"p" | b"progress" | b"d" | b"documentation"))
}

pub(super) fn compact(input: &[u8]) -> Option<Vec<u8>> {
    let finished = line_start(input, b"Finished in ")?;
    let summary = input[finished..]
        .split(|byte| *byte == b'\n')
        .find(|line| rspec_summary(line))?;
    let _ = summary;
    let start = line_start(input, b"Failures:").unwrap_or(finished);
    Some(input[start..].to_vec())
}

fn line_start(input: &[u8], prefix: &[u8]) -> Option<usize> {
    if input.starts_with(prefix) {
        return Some(0);
    }
    find_subslice(input, &[b"\n".as_slice(), prefix].concat()).map(|index| index + 1)
}

fn rspec_summary(line: &[u8]) -> bool {
    line.first().is_some_and(u8::is_ascii_digit)
        && (line
            .windows(b" example,".len())
            .any(|part| part == b" example,")
            || line
                .windows(b" examples,".len())
                .any(|part| part == b" examples,"))
        && (line
            .windows(b" failure".len())
            .any(|part| part == b" failure")
            || line
                .windows(b" failures".len())
                .any(|part| part == b" failures"))
}
