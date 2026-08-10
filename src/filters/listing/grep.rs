use std::collections::BTreeMap;

pub(super) fn route(argv: &[&[u8]]) -> bool {
    let args = crate::invocation_policy::options(argv);
    let mut positionals = 0usize;
    for argument in args {
        if argument.starts_with(b"--") {
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
        } else if argument.starts_with(b"-") && argument.len() > 1 {
            if !argument[1..].iter().all(|flag| b"EFGinvwHx".contains(flag)) {
                return false;
            }
        } else {
            positionals += 1;
        }
    }
    if let Some(terminator) = argv.iter().position(|argument| *argument == b"--") {
        positionals += argv.len().saturating_sub(terminator + 1);
    }
    positionals >= 3
}

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
        if first.len() < 3 {
            first.push(line);
        }
    }
    if line_count < 20 || groups.len() < 2 {
        return None;
    }
    let mut output = Vec::new();
    for (path, (count, first)) in groups {
        for line in first {
            output.extend_from_slice(line);
            output.push(b'\n');
        }
        if count > 3 {
            output.extend_from_slice(path);
            output.extend_from_slice(b": ... ");
            output.extend_from_slice((count - 3).to_string().as_bytes());
            output.extend_from_slice(b" more matches\n");
        }
    }
    Some(output)
}
