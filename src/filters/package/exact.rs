use super::*;

pub(super) fn requests_exact_output(argv: &[&[u8]]) -> bool {
    for argument in &argv[1..] {
        if *argument == b"--" {
            break;
        }
        if matches!(*argument, b"--help" | b"--version" | b"-h" | b"-V") {
            return true;
        }
        if matches!(
            *argument,
            b"--json"
                | b"--ndjson"
                | b"--parseable"
                | b"--porcelain"
                | b"--json-stream"
                | b"--format"
                | b"--reporter"
        ) || argument.starts_with(b"--json=")
            || argument.starts_with(b"--format=")
            || argument.starts_with(b"--reporter=")
        {
            return true;
        }
    }
    matches!(argv.get(1), Some(&b"help") | Some(&b"version"))
}

pub(super) fn trim_ascii(mut input: &[u8]) -> &[u8] {
    while input
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[1..];
    }
    trim_end(input)
}
