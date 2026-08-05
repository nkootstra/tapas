pub(super) enum Ownership {
    Missing,
    Modified,
    Valid(Value),
}

pub(super) struct HookOwnership {
    pub entry: Value,
    pub path: PathBuf,
    pub after_digest: Vec<u8>,
    pub before_existed: bool,
    pub backup_path: Option<PathBuf>,
}

pub(super) fn read_ownership(path: &Path) -> io::Result<Ownership> {
    let Some(content) = read_optional(path, MAX_CONFIG_BYTES)? else {
        return Ok(Ownership::Missing);
    };
    let rest = if let Some(rest) = content.strip_prefix(OWNERSHIP_HEADER) {
        rest
    } else if let Some(rest) = content.strip_prefix(b"tapas-setup-v2\n") {
        rest
    } else {
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
    write_record(path, entry)
}

pub(super) fn write_hook_ownership(
    path: &Path,
    target: Target,
    config_path: &Path,
    entry: &Value,
    after: &[u8],
    before_existed: bool,
    backup_path: Option<&Path>,
) -> io::Result<()> {
    let mut fields = vec![
        (b"kind".to_vec(), Value::String(b"hook".to_vec())),
        (
            b"target".to_vec(),
            Value::String(target.name().as_bytes().to_vec()),
        ),
        (
            b"path".to_vec(),
            Value::String(hex(config_path.as_os_str().as_bytes())),
        ),
        (b"entry".to_vec(), entry.clone()),
        (b"after".to_vec(), Value::String(digest(after).to_vec())),
        (b"before_existed".to_vec(), Value::Bool(before_existed)),
    ];
    if let Some(backup) = backup_path {
        fields.push((
            b"backup".to_vec(),
            Value::String(hex(backup.as_os_str().as_bytes())),
        ));
    }
    write_record(path, &Value::Object(fields))
}

pub(super) fn hook_ownership(value: &Value, expected_target: Target) -> Option<HookOwnership> {
    let Value::String(kind) = value.get(b"kind")? else {
        return None;
    };
    if kind != b"hook" {
        return None;
    }
    let Value::String(target) = value.get(b"target")? else {
        return None;
    };
    if target != expected_target.name().as_bytes() {
        return None;
    }
    let Value::String(path) = value.get(b"path")? else {
        return None;
    };
    let Value::String(after_digest) = value.get(b"after")? else {
        return None;
    };
    let Value::Bool(before_existed) = value.get(b"before_existed")? else {
        return None;
    };
    let backup_path = match value.get(b"backup") {
        Some(Value::String(path)) => Some(PathBuf::from(OsString::from_vec(unhex(path)?))),
        None => None,
        _ => return None,
    };
    Some(HookOwnership {
        entry: value.get(b"entry")?.clone(),
        path: PathBuf::from(OsString::from_vec(unhex(path)?)),
        after_digest: after_digest.clone(),
        before_existed: *before_existed,
        backup_path,
    })
}

pub(super) fn recorded_path(value: &Value) -> Option<PathBuf> {
    if matches!(value.get(b"kind"), Some(Value::String(kind)) if kind == b"hook") {
        let Value::String(path) = value.get(b"path")? else {
            return None;
        };
        return Some(PathBuf::from(OsString::from_vec(unhex(path)?)));
    }
    let Value::String(kind) = value.get(b"kind")? else {
        return None;
    };
    let Value::String(path) = value.get(b"path")? else {
        return None;
    };
    (kind == b"opencode-plugin").then(|| PathBuf::from(OsStr::from_bytes(path)))
}

pub(super) fn content_digest(input: &[u8]) -> Vec<u8> {
    digest(input).to_vec()
}

fn write_record(path: &Path, entry: &Value) -> io::Result<()> {
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
fn hex(input: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = Vec::with_capacity(input.len() * 2);
    for byte in input {
        output.push(HEX[(byte >> 4) as usize]);
        output.push(HEX[(byte & 0x0f) as usize]);
    }
    output
}

fn unhex(input: &[u8]) -> Option<Vec<u8>> {
    if !input.len().is_multiple_of(2) {
        return None;
    }
    input
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

use super::{MAX_CONFIG_BYTES, OWNERSHIP_HEADER, Target, Value, json, read_optional, write_atomic};
use std::ffi::{OsStr, OsString};
use std::fs::{self, Permissions};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
