use super::super::{append_line, find_subslice, strip_ansi_csi as strip_ansi};

pub(super) fn compact_prettier(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    let mut formatted = Vec::<Vec<u8>>::new();
    let mut total = 0usize;
    let mut output = Vec::with_capacity(stdout.len() + stderr.len());
    scan_prettier(stdout, &mut output, &mut formatted, &mut total);
    scan_prettier(stderr, &mut output, &mut formatted, &mut total);
    if total > 0 {
        output.extend_from_slice(b"formatted ");
        output.extend_from_slice(total.to_string().as_bytes());
        output.extend_from_slice(b": ");
        for (index, path) in formatted.iter().enumerate() {
            if index > 0 {
                output.extend_from_slice(b", ");
            }
            output.extend_from_slice(path);
        }
        if total > formatted.len() {
            output.extend_from_slice(b", ... (+");
            output.extend_from_slice((total - formatted.len()).to_string().as_bytes());
            output.push(b')');
        }
        output.push(b'\n');
    }
    output
}

fn scan_prettier(
    input: &[u8],
    output: &mut Vec<u8>,
    formatted: &mut Vec<Vec<u8>>,
    total: &mut usize,
) {
    for raw in input.split(|byte| *byte == b'\n') {
        let clean = strip_ansi(raw);
        let line = clean.trim_ascii_end();
        if line.is_empty() {
            continue;
        }
        if let Some((path, changed)) = parse_prettier_write(line) {
            if changed {
                *total += 1;
                if formatted.len() < 8 {
                    formatted.push(path.to_vec());
                }
            }
        } else if line.starts_with(b"[warn]")
            || line.starts_with(b"[error]")
            || line.starts_with(b"All matched files use Prettier")
            || find_subslice(line, b"Code style issues found").is_some()
            || find_subslice(line, b"No files matching").is_some()
        {
            append_line(output, line);
        }
    }
}

fn parse_prettier_write(mut line: &[u8]) -> Option<(&[u8], bool)> {
    if line.starts_with(b"[warn]") || line.starts_with(b"[error]") {
        return None;
    }
    let changed = if let Some(prefix) = line.strip_suffix(b" (unchanged)") {
        line = prefix;
        false
    } else {
        true
    };
    let before_ms = line.strip_suffix(b"ms")?;
    let space = before_ms.iter().rposition(|byte| *byte == b' ')?;
    let duration = &before_ms[space + 1..];
    (!duration.is_empty() && duration.iter().all(u8::is_ascii_digit) && space > 0)
        .then_some((&before_ms[..space], changed))
}
