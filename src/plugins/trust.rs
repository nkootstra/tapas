use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn sha256(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex(&digest.finalize()))
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

pub(super) struct ExecutableSnapshot {
    path: PathBuf,
}

impl ExecutableSnapshot {
    pub(super) fn create(source: &Path, expected: &str, directory: &Path) -> io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;

        let extension = source
            .extension()
            .and_then(OsStr::to_str)
            .map_or(String::new(), |extension| format!(".{extension}"));
        let mut source = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(source)?;
        let metadata = source.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o111 == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "plugin changed before snapshot",
            ));
        }
        let (path, mut destination) = loop {
            let sequence = SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!(
                ".plugin-exec-{}-{sequence}{extension}",
                std::process::id()
            ));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o500)
                .open(&path)
            {
                Ok(file) => break (path, file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };
        let result = (|| {
            let mut digest = Sha256::new();
            let mut buffer = [0_u8; 16 * 1024];
            loop {
                let read = source.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
                io::Write::write_all(&mut destination, &buffer[..read])?;
            }
            destination.sync_all()?;
            if hex(&digest.finalize()) != expected {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "plugin integrity changed before execution",
                ));
            }
            Ok(Self {
                path: fs::canonicalize(&path)?,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ExecutableSnapshot {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn trusted_plugin_path(path: &Path) -> io::Result<PathBuf> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "plugin must not be a symbolic link",
        ));
    }
    let path = fs::canonicalize(path)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "plugin must be a regular executable file",
        ));
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "plugin must be owned by the current user",
        ));
    }
    for ancestor in path.ancestors() {
        if fs::metadata(ancestor)?.permissions().mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "plugin and its path ancestors must not be group- or world-writable",
            ));
        }
    }
    Ok(path)
}

pub(super) fn valid_sha256(value: &OsStr) -> io::Result<String> {
    let value = value
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "SHA-256 must be UTF-8"))?;
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value.to_ascii_lowercase())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SHA-256 must contain exactly 64 hexadecimal characters",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecutableSnapshot, sha256};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    #[test]
    fn snapshot_executes_the_verified_bytes_after_the_source_is_replaced() {
        let directory =
            std::env::temp_dir().join(format!("tapas-snapshot-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let source = directory.join("plugin");
        fs::write(&source, b"#!/bin/sh\nprintf 'verified\\n'\n").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
        let digest = sha256(&source).unwrap();

        let snapshot = ExecutableSnapshot::create(&source, &digest, &directory).unwrap();
        fs::write(&source, b"#!/bin/sh\nprintf 'replaced\\n'\n").unwrap();

        let output = Command::new(snapshot.path()).output().unwrap();
        assert_eq!(output.stdout, b"verified\n");
        drop(snapshot);
        fs::remove_dir_all(directory).unwrap();
    }
}
