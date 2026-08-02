#[derive(Clone, Copy)]
struct DuRow<'a> {
    number: &'a [u8],
    unit: Option<u8>,
    bytes: u64,
    path: &'a [u8],
}

pub(super) fn matches_du(input: &[u8]) -> bool {
    let mut saw_any = false;
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if parse_du_line(line).is_none() {
            return false;
        }
        saw_any = true;
    }
    saw_any
}

fn parse_du_line(line: &[u8]) -> Option<DuRow<'_>> {
    let mut index = line.iter().position(|byte| !matches!(byte, b' ' | b'\t'))?;
    let number_start = index;
    let mut saw_dot = false;
    while index < line.len() {
        match line[index] {
            b'0'..=b'9' => index += 1,
            b'.' if !saw_dot => {
                saw_dot = true;
                index += 1;
            }
            _ => break,
        }
    }
    if index == number_start {
        return None;
    }
    let number = &line[number_start..index];
    let mut unit = None;
    if let Some(byte) = line.get(index).copied() {
        let upper = byte.to_ascii_uppercase();
        if matches!(upper, b'K' | b'M' | b'G' | b'T' | b'P' | b'E') {
            unit = Some(upper);
            index += 1;
            if line
                .get(index)
                .is_some_and(|byte| byte.eq_ignore_ascii_case(&b'B'))
            {
                index += 1;
            }
        }
    }
    if line.get(index) != Some(&b'\t') {
        return None;
    }
    let path = trim_ascii_end_space(&line[index + 1..]);
    if path.is_empty() {
        return None;
    }
    let bytes = du_bytes(number, unit)?;
    Some(DuRow {
        number,
        unit,
        bytes,
        path,
    })
}

fn du_bytes(number: &[u8], unit: Option<u8>) -> Option<u64> {
    let multiplier = match unit {
        None | Some(b'K') => 1024_u64,
        Some(b'M') => 1024_u64.pow(2),
        Some(b'G') => 1024_u64.pow(3),
        Some(b'T') => 1024_u64.pow(4),
        Some(b'P') => 1024_u64.pow(5),
        Some(b'E') => 1024_u64.pow(6),
        _ => return None,
    };
    let dot = number.iter().position(|byte| *byte == b'.');
    let integer = parse_u64(dot.map_or(number, |position| &number[..position]))?;
    let fraction = dot
        .and_then(|position| number.get(position + 1).copied())
        .filter(u8::is_ascii_digit)
        .map_or(0, |byte| u64::from(byte - b'0'));
    let tenths = integer.saturating_mul(10).saturating_add(fraction);
    Some(
        tenths
            .saturating_mul(multiplier / 10)
            .saturating_add(tenths.saturating_mul(multiplier % 10) / 10),
    )
}

fn parse_u64(input: &[u8]) -> Option<u64> {
    if input.is_empty() {
        return None;
    }
    input.iter().try_fold(0_u64, |value, byte| {
        byte.is_ascii_digit().then(|| {
            value
                .saturating_mul(10)
                .saturating_add(u64::from(*byte - b'0'))
        })
    })
}

pub(super) fn apply_du(input: &[u8], sort_descending: bool) -> Vec<u8> {
    let mut rows: Vec<_> = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(parse_du_line)
        .collect();
    if !sort_descending {
        let mut output = Vec::with_capacity(input.len());
        for row in rows {
            write_du_row(&mut output, row, 0);
        }
        return output;
    }

    rows.sort_by_key(|row| std::cmp::Reverse(row.bytes));
    let emitted = rows.len().min(10);
    let prefix_len = common_path_prefix(&rows[..emitted]);
    let mut output = Vec::with_capacity(input.len());
    if prefix_len > 2 {
        output.extend_from_slice(&rows[0].path[..prefix_len]);
        output.push(b'\n');
    }
    for row in &rows[..emitted] {
        write_du_row(
            &mut output,
            *row,
            if prefix_len > 2 { prefix_len } else { 0 },
        );
    }
    if rows.len() > 10 {
        let remaining = rows[10..]
            .iter()
            .fold(0_u64, |total, row| total.saturating_add(row.bytes));
        write_human_size(&mut output, remaining);
        output.extend_from_slice(b"\t(+");
        output.extend_from_slice((rows.len() - 10).to_string().as_bytes());
        output.extend_from_slice(b")\n");
    }
    output
}

fn common_path_prefix(rows: &[DuRow<'_>]) -> usize {
    if rows.len() <= 1 {
        return 0;
    }
    let mut prefix_len = rows[0].path.len();
    for row in &rows[1..] {
        prefix_len = rows[0].path[..prefix_len]
            .iter()
            .zip(row.path)
            .take_while(|(left, right)| left == right)
            .count();
    }
    while prefix_len > 0 && rows[0].path[prefix_len - 1] != b'/' {
        prefix_len -= 1;
    }
    prefix_len
}

fn write_du_row(output: &mut Vec<u8>, row: DuRow<'_>, strip_prefix: usize) {
    write_rounded_number(output, row.number);
    if let Some(unit) = row.unit {
        output.push(unit);
    }
    output.push(b'\t');
    output.extend_from_slice(&row.path[strip_prefix..]);
    output.push(b'\n');
}

fn write_rounded_number(output: &mut Vec<u8>, number: &[u8]) {
    if number.contains(&b'.') || number.len() <= 2 {
        output.extend_from_slice(number);
        return;
    }
    output.extend_from_slice(&number[..2]);
    output.resize(output.len() + number.len() - 2, b'0');
}

fn write_human_size(output: &mut Vec<u8>, bytes: u64) {
    let (unit, suffix) = if bytes >= 1024_u64.pow(4) {
        (1024_u64.pow(4), b'T')
    } else if bytes >= 1024_u64.pow(3) {
        (1024_u64.pow(3), b'G')
    } else if bytes >= 1024_u64.pow(2) {
        (1024_u64.pow(2), b'M')
    } else if bytes >= 1024 {
        (1024, b'K')
    } else {
        output.extend_from_slice(bytes.to_string().as_bytes());
        return;
    };
    let tenths = bytes / (unit / 10);
    output.extend_from_slice((tenths / 10).to_string().as_bytes());
    output.push(b'.');
    output.push(b'0' + (tenths % 10) as u8);
    output.push(suffix);
}
use super::pipe::trim_ascii_end_space;
