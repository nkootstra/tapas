pub(super) fn collapse_table(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let mut first = true;
        for field in line
            .split(|byte| matches!(byte, b' ' | b'\t'))
            .filter(|field| !field.is_empty())
        {
            if !first {
                output.push(b' ');
            }
            first = false;
            output.extend_from_slice(field);
        }
        output.push(b'\n');
    }
    output
}

pub(super) fn first_nonempty(input: &[u8]) -> Option<&[u8]> {
    input
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
}

pub(super) fn first_field(line: &[u8]) -> &[u8] {
    let line = line.trim_ascii_start();
    &line[..line
        .iter()
        .position(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
        .unwrap_or(line.len())]
}

pub(super) fn last_field(line: &[u8]) -> &[u8] {
    let line = line.trim_ascii_end();
    for index in (2..=line.len()).rev() {
        if line[index - 2..index] == *b"  " {
            return &line[index..];
        }
    }
    line
}

pub(super) fn strip_prefix_ignore_ascii_case<'a>(
    input: &'a [u8],
    prefix: &[u8],
) -> Option<&'a [u8]> {
    input
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &input[prefix.len()..])
}
