use std::collections::BTreeMap;

pub(super) fn route(argv: &[&[u8]]) -> bool {
    let args = before_terminator(argv);
    let mut positionals = 0usize;
    for argument in args.iter().skip(1) {
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
    let lines = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 20 {
        return None;
    }
    let mut groups: BTreeMap<&[u8], Vec<&[u8]>> = BTreeMap::new();
    for line in lines {
        let colon = line.iter().position(|byte| *byte == b':')?;
        let path = &line[..colon];
        if path.is_empty() || path.iter().any(|byte| byte.is_ascii_whitespace()) {
            return None;
        }
        groups.entry(path).or_default().push(line);
    }
    if groups.len() < 2 {
        return None;
    }
    let mut output = Vec::new();
    for (path, matches) in groups {
        for line in matches.iter().take(3) {
            output.extend_from_slice(line);
            output.push(b'\n');
        }
        if matches.len() > 3 {
            output.extend_from_slice(path);
            output.extend_from_slice(b": ... ");
            output.extend_from_slice((matches.len() - 3).to_string().as_bytes());
            output.extend_from_slice(b" more matches\n");
        }
    }
    Some(output)
}

fn before_terminator<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    let end = argv
        .iter()
        .position(|argument| *argument == b"--")
        .unwrap_or(argv.len());
    &argv[..end]
}
