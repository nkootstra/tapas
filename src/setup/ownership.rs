pub(super) enum Ownership {
    Missing,
    Modified,
    Valid(Value),
}

pub(super) fn read_ownership(path: &Path) -> io::Result<Ownership> {
    let Some(content) = read_optional(path, MAX_CONFIG_BYTES)? else {
        return Ok(Ownership::Missing);
    };
    let Some(rest) = content.strip_prefix(OWNERSHIP_HEADER) else {
        return Ok(Ownership::Modified);
    };
    let Some(newline) = rest.iter().position(|byte| *byte == b'\n') else {
        return Ok(Ownership::Modified);
    };
    if newline != 16 {
        return Ok(Ownership::Modified);
    }
    let payload = &rest[newline + 1..];
    if rest[..newline] != digest(payload) {
        return Ok(Ownership::Modified);
    }
    let Ok(entry @ Value::Object(_)) = json::parse(payload) else {
        return Ok(Ownership::Modified);
    };
    Ok(Ownership::Valid(entry))
}

pub(super) fn write_ownership(path: &Path, entry: &Value) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ownership path has no parent",
        ));
    };
    fs::create_dir_all(parent)?;
    if let Some(root) = parent.parent() {
        fs::set_permissions(root, Permissions::from_mode(0o700))?;
    }
    fs::set_permissions(parent, Permissions::from_mode(0o700))?;
    let payload = json::serialize(entry);
    let mut content = OWNERSHIP_HEADER.to_vec();
    content.extend_from_slice(&digest(&payload));
    content.push(b'\n');
    content.extend_from_slice(&payload);
    write_atomic(path, &content, 0o600)
}

fn digest(input: &[u8]) -> [u8; 16] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for byte in input {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100_0000_01b3);
    }
    let mut output = [0_u8; 16];
    for index in (0..output.len()).rev() {
        output[index] = HEX[(value & 0x0f) as usize];
        value >>= 4;
    }
    output
}
use super::{MAX_CONFIG_BYTES, OWNERSHIP_HEADER, Value, json, read_optional, write_atomic};
use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
