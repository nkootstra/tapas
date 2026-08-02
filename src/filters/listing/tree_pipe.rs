pub(super) fn unicode_tree_line(line: &[u8]) -> bool {
    line.len() >= 3 && line[0] == 0xe2 && line[1] == 0x94 && matches!(line[2], 0x9c | 0x94 | 0x82)
}

pub(super) fn parse_ascii_tree_line(line: &[u8]) -> Option<(usize, &[u8])> {
    let mut index = 0;
    let mut depth = 0;
    while line.get(index..index + 4) == Some(b"|   ") || line.get(index..index + 4) == Some(b"    ")
    {
        depth += 1;
        index += 4;
    }
    if !matches!(line.get(index..index + 4), Some(b"|-- ") | Some(b"`-- ")) {
        return None;
    }
    let name = line[index + 4..].trim_ascii();
    (!name.is_empty()).then_some((depth + 1, name))
}

pub(super) fn tree_prefix_len(line: &[u8]) -> usize {
    let mut index = 0;
    while index < line.len() {
        match line[index] {
            b' ' => index += 1,
            0xc2 if line.get(index + 1) == Some(&0xa0) => index += 2,
            0xe2 if line.get(index + 1) == Some(&0x94) && index + 2 < line.len() => {
                index += 3;
            }
            _ => break,
        }
    }
    index
}

pub(super) fn tree_depth(prefix: &[u8]) -> usize {
    let mut columns = 0;
    let mut index = 0;
    while index < prefix.len() {
        if prefix[index] == 0xe2 && prefix.get(index + 1) == Some(&0x94) && index + 2 < prefix.len()
        {
            index += 3;
        } else if prefix[index] == 0xc2 && prefix.get(index + 1) == Some(&0xa0) {
            index += 2;
        } else {
            index += 1;
        }
        columns += 1;
    }
    columns / 4
}

pub(super) fn apply_tree_pipe(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous_depth = 0;
    let mut index = 0;
    while index < input.len() {
        let end = input[index..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(input.len(), |relative| index + relative);
        let line = &input[index..end];
        let prefix_len = tree_prefix_len(line);
        let name = &line[prefix_len..];
        let depth = tree_depth(&line[..prefix_len]);
        if !name.is_empty() {
            if depth == previous_depth && depth > 0 {
                output.push(b'~');
                output.extend_from_slice(name);
            } else {
                for _ in 0..depth {
                    output.extend_from_slice(b"  ");
                }
                output.extend_from_slice(name);
                previous_depth = depth;
            }
        } else if prefix_len == 0 && !line.is_empty() {
            output.extend_from_slice(line);
            previous_depth = 0;
        }
        if end < input.len() {
            output.push(b'\n');
            index = end + 1;
        } else {
            break;
        }
    }
    output
}
