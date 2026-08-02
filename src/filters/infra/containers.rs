pub(super) fn compact_docker_ps(input: &[u8]) -> Vec<u8> {
    let mut lines = input.split(|byte| *byte == b'\n');
    let header = lines.next().unwrap_or_default();
    let status_column = find_subslice(header, b"STATUS").unwrap_or(0);
    let names_column = find_subslice(header, b"NAMES")
        .or_else(|| find_subslice(header, b"NAME"))
        .unwrap_or(0);
    let compose = names_column == 0 && header.starts_with(b"NAME");
    let rows = lines.filter(|line| !line.is_empty()).collect::<Vec<_>>();
    let up = rows
        .iter()
        .filter(|line| {
            line.get(status_column..)
                .is_some_and(|status| status.trim_ascii_start().starts_with(b"Up"))
        })
        .count();
    let state: &[u8] = if rows.is_empty() || up == 0 {
        b"none".as_slice()
    } else if up == rows.len() {
        b"up"
    } else {
        b"m"
    };
    let mut output = b"d".to_vec();
    output.extend_from_slice(rows.len().to_string().as_bytes());
    output.extend_from_slice(state);
    for line in rows {
        let name = if compose {
            first_field(line)
        } else if names_column > 0 && names_column < line.len() {
            line[names_column..].trim_ascii()
        } else {
            last_field(line)
        };
        if name.is_empty() {
            continue;
        }
        output.push(b' ');
        output.extend_from_slice(name);
        if compose {
            let rest = line
                .trim_ascii_start()
                .get(name.len()..)
                .unwrap_or_default();
            let image = first_field(rest);
            if !image.is_empty() {
                output.push(b'(');
                output.extend_from_slice(image);
                output.push(b')');
            }
        }
    }
    output.push(b'\n');
    output
}

pub(super) fn is_docker_images(command: &[u8], argv: &[&[u8]]) -> bool {
    command == b"docker" && matches!(argv.get(1), Some(&b"images") | Some(&b"image"))
}

pub(super) fn matches_docker_images(input: &[u8]) -> bool {
    first_nonempty(input).is_some_and(|header| {
        header.starts_with(b"REPOSITORY")
            && find_subslice(header, b"TAG").is_some()
            && find_subslice(header, b"IMAGE ID").is_some()
            && find_subslice(header, b"SIZE").is_some()
    })
}

pub(super) fn compact_docker_images(input: &[u8]) -> Vec<u8> {
    let mut lines = input.split(|byte| *byte == b'\n');
    let header = lines.next().unwrap_or_default();
    let tag = find_subslice(header, b"TAG").unwrap_or(0);
    let image_id = find_subslice(header, b"IMAGE ID").unwrap_or(0);
    let size = find_subslice(header, b"SIZE").unwrap_or(0);
    let mut named = Vec::<(&[u8], &[u8], &[u8])>::new();
    let mut dangling = 0usize;
    for line in lines.filter(|line| !line.is_empty()) {
        if tag >= line.len() || image_id > line.len() || size >= line.len() {
            continue;
        }
        let repository = line[..tag].trim_ascii();
        let version = line[tag..image_id].trim_ascii();
        let bytes = line[size..].trim_ascii();
        if repository.is_empty() || version.is_empty() || bytes.is_empty() {
            continue;
        }
        if repository == b"<none>" || version == b"<none>" {
            dangling += 1;
        } else {
            named.push((repository, version, bytes));
        }
    }
    let total = named.len() + dangling;
    let mut output = b"images ".to_vec();
    output.extend_from_slice(total.to_string().as_bytes());
    if total > 0 {
        output.push(b':');
        for (repository, version, bytes) in named.iter().take(8) {
            output.push(b' ');
            output.extend_from_slice(repository);
            output.push(b':');
            output.extend_from_slice(version);
            output.push(b'(');
            output.extend_from_slice(bytes);
            output.push(b')');
        }
        if dangling > 0 {
            output.extend_from_slice(b" dangling x");
            output.extend_from_slice(dangling.to_string().as_bytes());
        }
        if named.len() > 8 {
            output.extend_from_slice(b" (+");
            output.extend_from_slice((named.len() - 8).to_string().as_bytes());
            output.push(b')');
        }
    }
    output.push(b'\n');
    output
}

pub(super) fn matches_kubectl(input: &[u8]) -> bool {
    first_nonempty(input).is_some_and(|header| {
        header.starts_with(b"NAME")
            && find_subslice(header, b"READY").is_some()
            && find_subslice(header, b"STATUS").is_some()
    })
}

pub(super) fn compact_kubectl(input: &[u8]) -> Vec<u8> {
    let mut lines = input.split(|byte| *byte == b'\n');
    let header = lines.next().unwrap_or_default();
    let ready_column = find_subslice(header, b"READY").unwrap_or(0);
    let status_column = find_subslice(header, b"STATUS").unwrap_or(0);
    let rows = lines.filter(|line| !line.is_empty()).collect::<Vec<_>>();
    let healthy = rows
        .iter()
        .filter(|line| healthy_pod(line, ready_column, status_column))
        .count();
    let aggregate = if rows.is_empty() || healthy == 0 {
        b'n'
    } else if healthy == rows.len() {
        b'r'
    } else {
        b'm'
    };
    let mut output = b"k".to_vec();
    output.extend_from_slice(rows.len().to_string().as_bytes());
    output.push(aggregate);
    for line in rows {
        let name = first_field(line);
        if name.is_empty() {
            continue;
        }
        output.push(b' ');
        output.extend_from_slice(name);
        if aggregate == b'r' {
            continue;
        }
        let ready = field_at(line, ready_column);
        output.push(b'(');
        output.extend_from_slice(ready);
        if !healthy_pod(line, ready_column, status_column) {
            output.push(b',');
            output.extend_from_slice(field_at(line, status_column));
        }
        output.push(b')');
    }
    output.push(b'\n');
    output
}

fn healthy_pod(line: &[u8], ready: usize, status: usize) -> bool {
    field_at(line, status) == b"Running" && ready_full(field_at(line, ready))
}

fn ready_full(ready: &[u8]) -> bool {
    let Some(slash) = ready.iter().position(|byte| *byte == b'/') else {
        return false;
    };
    let (left, right) = (&ready[..slash], &ready[slash + 1..]);
    !left.is_empty() && left != b"0" && left == right
}

fn field_at(line: &[u8], column: usize) -> &[u8] {
    if column >= line.len() {
        return b"";
    }
    let mut cursor = column;
    while line
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    if column > 0 && cursor == column && !matches!(line[column], b' ' | b'\t') {
        while cursor > 0 && !matches!(line[cursor - 1], b' ' | b'\t') {
            cursor -= 1;
        }
    }
    let start = cursor;
    while line
        .get(cursor)
        .is_some_and(|byte| !matches!(byte, b' ' | b'\t'))
    {
        cursor += 1;
    }
    &line[start..cursor]
}
use super::find_subslice;
use super::table::{first_field, first_nonempty, last_field};
