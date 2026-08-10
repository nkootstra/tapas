use super::append_line;

pub(super) fn compact(argv: &[&[u8]], stdout: &[u8]) -> Option<Vec<u8>> {
    match argv.get(1).copied()? {
        b"list" if argv.len() == 2 && matches_table(stdout, b"NAME") => compact_list(stdout),
        b"history" if argv.len() == 3 && matches_table(stdout, b"REVISION") => {
            compact_history(stdout)
        }
        b"status" if argv.len() == 3 && matches_status(stdout) => Some(compact_status(stdout)),
        _ => None,
    }
}

fn matches_table(input: &[u8], first: &[u8]) -> bool {
    input
        .split(|byte| *byte == b'\n')
        .next()
        .is_some_and(|header| header.trim_ascii_start().starts_with(first))
}

fn compact_list(input: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    for line in input.split(|byte| *byte == b'\n').skip(1) {
        let columns = fields(line);
        if columns.is_empty() {
            continue;
        }
        if columns.len() < 9 {
            return None;
        }
        output.extend_from_slice(columns[0]);
        output.push(b' ');
        output.extend_from_slice(columns[1]);
        output.extend_from_slice(b" r");
        output.extend_from_slice(columns[2]);
        for column in &columns[columns.len() - 3..] {
            output.push(b' ');
            output.extend_from_slice(column);
        }
        output.push(b'\n');
    }
    (!output.is_empty()).then_some(output)
}

fn compact_history(input: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    for line in input.split(|byte| *byte == b'\n').skip(1) {
        let columns = fields(line);
        if columns.is_empty() {
            continue;
        }
        let status = columns.iter().position(|column| is_status(column))?;
        if status + 2 >= columns.len() {
            return None;
        }
        output.extend_from_slice(b"r");
        output.extend_from_slice(columns[0]);
        for column in &columns[status..] {
            output.push(b' ');
            output.extend_from_slice(column);
        }
        output.push(b'\n');
    }
    (!output.is_empty()).then_some(output)
}

fn matches_status(input: &[u8]) -> bool {
    input.starts_with(b"NAME:")
        && input
            .split(|byte| *byte == b'\n')
            .any(|line| line.starts_with(b"STATUS:"))
}

fn compact_status(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut notes = false;
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if line == b"NOTES:" {
            notes = true;
        }
        if notes
            || [b"NAME:".as_slice(), b"NAMESPACE:", b"STATUS:", b"REVISION:"]
                .iter()
                .any(|prefix| line.starts_with(prefix))
        {
            append_line(&mut output, line);
        }
    }
    output
}

fn fields(line: &[u8]) -> Vec<&[u8]> {
    line.split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty())
        .collect()
}

fn is_status(value: &[u8]) -> bool {
    matches!(
        value,
        b"deployed"
            | b"failed"
            | b"superseded"
            | b"uninstalled"
            | b"pending-install"
            | b"pending-upgrade"
            | b"pending-rollback"
    )
}
