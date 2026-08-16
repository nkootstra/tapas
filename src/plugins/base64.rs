use std::io;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn encode(input: &[u8]) -> String {
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let word = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((word >> 18) & 63) as usize] as char);
        output.push(ALPHABET[((word >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((word >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(word & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

pub(crate) fn decode(input: &[u8]) -> io::Result<Vec<u8>> {
    if !input.len().is_multiple_of(4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid base64 length",
        ));
    }
    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    for (index, chunk) in input.chunks_exact(4).enumerate() {
        let final_chunk = index + 1 == input.len() / 4;
        let padding = usize::from(chunk[3] == b'=') + usize::from(chunk[2] == b'=');
        if padding > 0 && !final_chunk || padding == 1 && chunk[2] == b'=' || padding > 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid base64 padding",
            ));
        }
        let a = value(chunk[0])?;
        let b = value(chunk[1])?;
        let c = if chunk[2] == b'=' {
            0
        } else {
            value(chunk[2])?
        };
        let d = if chunk[3] == b'=' {
            0
        } else {
            value(chunk[3])?
        };
        if padding == 2 && b & 0x0f != 0 || padding == 1 && c & 0x03 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "non-canonical base64",
            ));
        }
        let word = (u32::from(a) << 18) | (u32::from(b) << 12) | (u32::from(c) << 6) | u32::from(d);
        output.push((word >> 16) as u8);
        if padding < 2 {
            output.push((word >> 8) as u8);
        }
        if padding == 0 {
            output.push(word as u8);
        }
    }
    Ok(output)
}

fn value(byte: u8) -> io::Result<u8> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid base64 character",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn standard_vectors_round_trip() {
        for (raw, encoded) in [
            (b"".as_slice(), ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"\0\xff", "AP8="),
        ] {
            assert_eq!(encode(raw), encoded);
            assert_eq!(decode(encoded.as_bytes()).unwrap(), raw);
        }
    }

    #[test]
    fn rejects_invalid_and_noncanonical_input() {
        for input in [b"A===".as_slice(), b"Zh==", b"AA=A", b"!!!!", b"A"] {
            assert!(decode(input).is_err(), "{:?}", input);
        }
    }
}
