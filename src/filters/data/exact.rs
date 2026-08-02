pub(super) fn gh_wants_data_output(argv: &[&[u8]]) -> bool {
    argv.iter().any(|argument| {
        matches!(*argument, b"--json" | b"--jq")
            || argument.starts_with(b"--json=")
            || argument.starts_with(b"--jq=")
    })
}

pub mod sigil_rle {
    const PREFIX_LEN: usize = 16;
    const SIGIL: u8 = 0x01;

    pub fn encode(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut previous_prefix = &[][..];
        let mut first = true;
        let mut offset = 0usize;
        while offset < input.len() {
            let relative_end = input[offset..].iter().position(|byte| *byte == b'\n');
            let end = relative_end.map_or(input.len(), |index| offset + index);
            let line = &input[offset..end];
            if line.first() == Some(&SIGIL) {
                output.push(SIGIL);
                output.extend_from_slice(line);
            } else {
                let prefix_len = line.len().min(PREFIX_LEN);
                let prefix = &line[..prefix_len];
                let can_elide = !first
                    && prefix_len == PREFIX_LEN
                    && prefix == previous_prefix
                    && (line.len() == PREFIX_LEN || line[PREFIX_LEN] != SIGIL);
                if can_elide {
                    output.push(SIGIL);
                    output.extend_from_slice(&line[PREFIX_LEN..]);
                } else {
                    output.extend_from_slice(line);
                }
            }
            previous_prefix = &line[..line.len().min(PREFIX_LEN)];
            if end < input.len() {
                output.push(b'\n');
                offset = end + 1;
            } else {
                offset = end;
            }
            first = false;
        }
        output
    }

    pub fn decode(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut previous_prefix = [0u8; PREFIX_LEN];
        let mut previous_len = 0usize;
        let mut offset = 0usize;
        while offset < input.len() {
            let relative_end = input[offset..].iter().position(|byte| *byte == b'\n');
            let end = relative_end.map_or(input.len(), |index| offset + index);
            let line = &input[offset..end];
            if line.starts_with(&[SIGIL, SIGIL]) {
                let decoded = &line[1..];
                output.extend_from_slice(decoded);
                previous_len = decoded.len().min(PREFIX_LEN);
                previous_prefix[..previous_len].copy_from_slice(&decoded[..previous_len]);
            } else if line.first() == Some(&SIGIL) {
                output.extend_from_slice(&previous_prefix[..previous_len]);
                output.extend_from_slice(&line[1..]);
            } else {
                output.extend_from_slice(line);
                previous_len = line.len().min(PREFIX_LEN);
                previous_prefix[..previous_len].copy_from_slice(&line[..previous_len]);
            }
            if end < input.len() {
                output.push(b'\n');
                offset = end + 1;
            } else {
                offset = end;
            }
        }
        output
    }
}

pub mod ws_rle {
    use super::FilterError;

    const SIGIL: u8 = 0x01;
    const MIN_RUN: usize = 17;
    const MAX_RUN: usize = 255;

    pub fn encode(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut index = 0usize;
        while index < input.len() {
            if input[index] == SIGIL {
                output.extend_from_slice(&[SIGIL, 0]);
                index += 1;
            } else if input[index] == b' ' {
                let mut after = index;
                while after < input.len() && input[after] == b' ' {
                    after += 1;
                }
                let mut run = after - index;
                if run < MIN_RUN {
                    output.extend_from_slice(&input[index..after]);
                } else {
                    while run > 0 {
                        let chunk = run.min(MAX_RUN);
                        if chunk < MIN_RUN {
                            output.extend(std::iter::repeat_n(b' ', chunk));
                        } else {
                            output.extend_from_slice(&[SIGIL, chunk as u8]);
                        }
                        run -= chunk;
                    }
                }
                index = after;
            } else {
                output.push(input[index]);
                index += 1;
            }
        }
        output
    }

    pub fn decode(input: &[u8]) -> Result<Vec<u8>, FilterError> {
        let mut output = Vec::with_capacity(input.len());
        let mut index = 0usize;
        while index < input.len() {
            if input[index] != SIGIL {
                output.push(input[index]);
                index += 1;
                continue;
            }
            let length = *input.get(index + 1).ok_or(FilterError::InvalidInput)?;
            if length == 0 {
                output.push(SIGIL);
            } else if usize::from(length) >= MIN_RUN {
                output.extend(std::iter::repeat_n(b' ', usize::from(length)));
            } else {
                return Err(FilterError::InvalidInput);
            }
            index += 2;
        }
        Ok(output)
    }
}
use super::FilterError;
