pub(super) fn trim_ascii(mut input: &[u8]) -> &[u8] {
    while input
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
    {
        input = &input[1..];
    }
    trim_end(input)
}
use super::trim_end;
