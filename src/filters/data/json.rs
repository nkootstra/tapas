pub(super) fn compact_json(input: &[u8]) -> Option<Vec<u8>> {
    let input = trim_bom_and_space(input);
    if !is_single_top_level_container(input) {
        return None;
    }

    let mut output = Vec::with_capacity(input.len() + 1);
    let mut in_string = false;
    let mut escaped = false;
    for &byte in input {
        if in_string {
            output.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
            output.push(byte);
        } else if !byte.is_ascii_whitespace() {
            output.push(byte);
        }
    }
    output.push(b'\n');
    Some(output)
}

fn is_single_top_level_container(input: &[u8]) -> bool {
    let Some((&root, _)) = input.split_first() else {
        return false;
    };
    let expected_root = match root {
        b'{' => b'}',
        b'[' => b']',
        _ => return false,
    };
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for (index, &byte) in input.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.pop() != Some(byte) {
                    return false;
                }
                if stack.is_empty() {
                    return byte == expected_root && index + 1 == input.len();
                }
            }
            _ => {}
        }
    }
    false
}

fn trim_bom_and_space(input: &[u8]) -> &[u8] {
    let input = input.trim_ascii();
    input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input)
}
