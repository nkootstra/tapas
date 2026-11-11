use std::collections::BTreeMap;

pub(super) fn route(argv: &[&[u8]]) -> bool {
    let mut positionals = 0usize;
    let mut after_terminator = false;
    for argument in argv.get(1..).unwrap_or_default() {
        if !after_terminator && *argument == b"--" {
            after_terminator = true;
        } else if !after_terminator && argument.starts_with(b"--") {
            if !matches!(
                *argument,
                b"--extended-regexp"
                    | b"--fixed-strings"
                    | b"--basic-regexp"
                    | b"--ignore-case"
                    | b"--line-number"
                    | b"--with-filename"
                    | b"--word-regexp"
                    | b"--line-regexp"
                    | b"--invert-match"
            ) {
                return false;
            }
        } else if !after_terminator && argument.starts_with(b"-") && argument.len() > 1 {
            if !argument[1..].iter().all(|flag| b"EFGinvwHx".contains(flag)) {
                return false;
            }
        } else {
            // The compact output format also uses ':' between the filename and
            // match. Without escaping, a target containing ':' is ambiguous.
            // Keep such invocations byte-exact instead of guessing the path.
            if positionals > 0 && argument.contains(&b':') {
                return false;
            }
            positionals += 1;
        }
    }
    positionals >= 3
}

/// Below this many matching lines a multi-file search is returned unchanged:
/// it is cheap to read, and agents often want every hit.
const MIN_COMPACT_LINES: usize = 200;
/// Matches kept per file once the search is large enough to compact.
const KEEP_PER_FILE: usize = 10;

pub(super) fn compact(input: &[u8]) -> Option<Vec<u8>> {
    let mut line_count = 0usize;
    let mut groups: BTreeMap<&[u8], (usize, Vec<&[u8]>)> = BTreeMap::new();
    for line in input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        line_count += 1;
        let colon = line.iter().position(|byte| *byte == b':')?;
        let path = &line[..colon];
        if path.is_empty() || path.iter().any(|byte| byte.is_ascii_whitespace()) {
            return None;
        }
        let (count, first) = groups.entry(path).or_default();
        *count += 1;
        if first.len() < KEEP_PER_FILE {
            first.push(line);
        }
    }
    if line_count < MIN_COMPACT_LINES || groups.len() < 2 {
        return None;
    }
    let mut output = Vec::new();
    for (path, (count, first)) in groups {
        for line in first {
            output.extend_from_slice(line);
            output.push(b'\n');
        }
        if count > KEEP_PER_FILE {
            output.extend_from_slice(path);
            output.extend_from_slice(b": ... ");
            output.extend_from_slice((count - KEEP_PER_FILE).to_string().as_bytes());
            output.extend_from_slice(b" more matches\n");
        }
    }
    Some(output)
}
