use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::filters::contains_ignore_ascii_case;

use super::super::storage::read_optional;
use super::super::{MAX_CONFIG_BYTES, lossless};

pub(super) struct Predecessor {
    pub(super) path: PathBuf,
    pub(super) recognized: bool,
    pub(super) content: Vec<u8>,
}

pub(super) fn opencode_predecessors(plugin_dir: &Path) -> io::Result<Vec<Predecessor>> {
    let candidates = [
        (plugin_dir.join("rtk.ts"), PredecessorKind::RtkPlugin),
        (
            plugin_dir.join("smll-proxy.ts"),
            PredecessorKind::SmllPlugin,
        ),
        (
            plugin_dir.join("smll-proxy.js"),
            PredecessorKind::SmllPlugin,
        ),
        (
            plugin_dir.join("smll-proxy/index.ts"),
            PredecessorKind::SmllPlugin,
        ),
        (
            plugin_dir.join("smll-proxy/package.json"),
            PredecessorKind::SmllPackage,
        ),
    ];
    let mut found = Vec::new();
    for (path, kind) in candidates {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                found.push(Predecessor {
                    path,
                    recognized: false,
                    content: Vec::new(),
                });
            }
            Ok(_) => {
                let content = read_optional(&path, MAX_CONFIG_BYTES)?.unwrap_or_default();
                found.push(Predecessor {
                    recognized: predecessor_content_recognized(&content, kind),
                    path,
                    content,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(found)
}

#[derive(Clone, Copy)]
enum PredecessorKind {
    RtkPlugin,
    SmllPlugin,
    SmllPackage,
}

fn predecessor_content_recognized(content: &[u8], kind: PredecessorKind) -> bool {
    match kind {
        PredecessorKind::RtkPlugin => {
            contains_ignore_ascii_case(content, b"tool.execute.before")
                && contains_ignore_ascii_case(content, b"rtk")
                && (contains_ignore_ascii_case(content, b"RtkOpenCodePlugin")
                    || contains_ignore_ascii_case(content, b"rtk rewrite"))
        }
        PredecessorKind::SmllPlugin => {
            contains_ignore_ascii_case(content, b"tool.execute.before")
                && contains_ignore_ascii_case(content, b"smll")
                && (contains_ignore_ascii_case(content, b"SmllProxyPlugin")
                    || contains_ignore_ascii_case(content, b"smll-proxy"))
        }
        PredecessorKind::SmllPackage => {
            contains_ignore_ascii_case(content, b"\"name\"")
                && contains_ignore_ascii_case(content, b"smll-proxy")
                && contains_ignore_ascii_case(content, b"\"main\"")
                && contains_ignore_ascii_case(content, b"index.ts")
        }
    }
}

pub(super) fn smll_opencode_ownership_recognized(
    content: &[u8],
    index_digest: &[u8; 16],
    package_digest: &[u8; 16],
) -> bool {
    let Some(rest) = content.strip_prefix(b"smll-setup-v1\n") else {
        return false;
    };
    let Some(newline) = rest.iter().position(|byte| *byte == b'\n') else {
        return false;
    };
    let envelope_digest = &rest[..newline];
    let payload = &rest[newline + 1..];
    payload.len() == 33
        && payload[16] == b'\n'
        && envelope_digest == smll_digest(payload)
        && payload[..16] == index_digest[..]
        && payload[17..] == package_digest[..]
}

pub(super) fn smll_digest(input: &[u8]) -> [u8; 16] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = wyhash(input);
    let mut output = [0_u8; 16];
    for byte in output.iter_mut().rev() {
        *byte = HEX[(value & 0x0f) as usize];
        value >>= 4;
    }
    output
}

fn wyhash(input: &[u8]) -> u64 {
    const SECRET: [u64; 4] = [
        0xa076_1d64_78bd_642f,
        0xe703_7ed1_a0b4_28db,
        0x8ebc_6af0_9c88_c6e3,
        0x5899_65cc_7537_4cc3,
    ];
    let initial = wyhash_mix(SECRET[0], SECRET[1]);
    let mut state = [initial; 3];
    let (mut a, mut b);

    if input.len() <= 16 {
        if input.len() >= 4 {
            let end = input.len() - 4;
            let quarter = (input.len() >> 3) << 2;
            a = (read_le(input, 0, 4) << 32) | read_le(input, quarter, 4);
            b = (read_le(input, end, 4) << 32) | read_le(input, end - quarter, 4);
        } else if input.is_empty() {
            a = 0;
            b = 0;
        } else {
            a = (u64::from(input[0]) << 16)
                | (u64::from(input[input.len() >> 1]) << 8)
                | u64::from(input[input.len() - 1]);
            b = 0;
        }
    } else {
        let mut offset = 0;
        if input.len() >= 48 {
            while offset + 48 < input.len() {
                for index in 0..3 {
                    let start = offset + index * 16;
                    state[index] = wyhash_mix(
                        read_le(input, start, 8) ^ SECRET[index + 1],
                        read_le(input, start + 8, 8) ^ state[index],
                    );
                }
                offset += 48;
            }
            state[0] ^= state[1] ^ state[2];
        }
        while offset + 16 < input.len() {
            state[0] = wyhash_mix(
                read_le(input, offset, 8) ^ SECRET[1],
                read_le(input, offset + 8, 8) ^ state[0],
            );
            offset += 16;
        }
        a = read_le(input, input.len() - 16, 8);
        b = read_le(input, input.len() - 8, 8);
    }

    a ^= SECRET[1];
    b ^= state[0];
    let product = u128::from(a) * u128::from(b);
    a = product as u64;
    b = (product >> 64) as u64;
    wyhash_mix(a ^ SECRET[0] ^ input.len() as u64, b ^ SECRET[1])
}

fn read_le(input: &[u8], start: usize, len: usize) -> u64 {
    input[start..start + len]
        .iter()
        .enumerate()
        .fold(0, |value, (index, byte)| {
            value | (u64::from(*byte) << (index * 8))
        })
}

fn wyhash_mix(left: u64, right: u64) -> u64 {
    let product = u128::from(left) * u128::from(right);
    product as u64 ^ (product >> 64) as u64
}

pub(super) fn opencode_config_without_predecessors(
    input: &[u8],
    plugin_dir: &Path,
) -> Result<Vec<u8>, ()> {
    let smll_directory = plugin_dir.join("smll-proxy");
    let values = vec![
        smll_directory.as_os_str().as_encoded_bytes().to_vec(),
        plugin_dir
            .join("smll-proxy.ts")
            .as_os_str()
            .as_encoded_bytes()
            .to_vec(),
        plugin_dir
            .join("smll-proxy.js")
            .as_os_str()
            .as_encoded_bytes()
            .to_vec(),
        plugin_dir
            .join("rtk.ts")
            .as_os_str()
            .as_encoded_bytes()
            .to_vec(),
    ];
    lossless::remove_root_array_strings(input, b"plugin", &values).map(|(bytes, _)| bytes)
}

pub(super) fn contains_predecessor_marker(input: &[u8]) -> bool {
    [b"smll-proxy".as_slice(), b"rtk.ts", b"run-toolkit"]
        .iter()
        .any(|marker| contains_ignore_ascii_case(input, marker))
}

pub(super) fn opencode_external_conflicts(plugin_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join(".opencode/plugins"));
        roots.push(cwd.join("opencode/plugins"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".opencode/plugins"));
    }
    if let Some(custom) = std::env::var_os("OPENCODE_CONFIG_DIR").filter(|value| !value.is_empty())
    {
        roots.push(PathBuf::from(custom).join("plugins"));
    }
    let managed = fs::canonicalize(plugin_dir).unwrap_or_else(|_| plugin_dir.to_path_buf());
    let mut conflicts = Vec::new();
    for root in roots {
        if fs::canonicalize(&root).unwrap_or_else(|_| root.clone()) == managed {
            continue;
        }
        conflicts.extend(
            opencode_predecessors(&root)?
                .into_iter()
                .map(|item| item.path),
        );
    }
    Ok(conflicts)
}
