pub(super) fn read_optional(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let length = file.metadata()?.len();
    if length > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    let mut content = Vec::with_capacity(length.min(limit) as usize);
    file.take(limit + 1).read_to_end(&mut content)?;
    if content.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    Ok(Some(content))
}

pub(super) fn reject_symlink(path: &Path, stderr: &mut dyn Write) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            stderr.write_all(b"tapas agent setup: symbolic-link configuration is not supported; configuration left untouched\n")?;
            Ok(true)
        }
        Ok(metadata) if !metadata.is_file() => {
            stderr.write_all(b"tapas agent setup: managed paths must be regular files; configuration left untouched\n")?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn write_unique_backup(
    path: &Path,
    existing: Option<&[u8]>,
) -> io::Result<Option<PathBuf>> {
    let Some(existing) = existing else {
        return Ok(None);
    };
    let base = backup_path(path);
    let mut sequence = 0_u32;
    loop {
        let candidate = if sequence == 0 {
            base.clone()
        } else {
            let mut name = base
                .file_name()
                .unwrap_or(OsStr::new("settings.json.bak.tapas"))
                .to_os_string();
            name.push(format!(".{sequence}"));
            base.with_file_name(name)
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&candidate)
        {
            Ok(mut file) => {
                let result = (|| {
                    file.write_all(existing)?;
                    file.sync_all()?;
                    if let Some(parent) = candidate.parent() {
                        File::open(parent)?.sync_all()?;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    drop(file);
                    let _ = fs::remove_file(&candidate);
                    return Err(error);
                }
                return Ok(Some(candidate));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => sequence += 1,
            Err(error) => return Err(error),
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .unwrap_or(OsStr::new("settings.json"))
        .to_os_string();
    name.push(".bak.tapas");
    path.with_file_name(name)
}

pub(super) fn restore_optional(path: &Path, existing: Option<&[u8]>) -> io::Result<()> {
    if let Some(existing) = existing {
        write_atomic(path, existing, existing_mode(path, 0o600))
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

pub(super) fn existing_mode(path: &Path, default: u32) -> u32 {
    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o777)
        .unwrap_or(default)
}

pub(super) fn write_atomic(path: &Path, content: &[u8], mode: u32) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output path has no parent"))?;
    fs::create_dir_all(parent)?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = OsString::from(".");
    temp_name.push(path.file_name().unwrap_or(OsStr::new("tapas")));
    temp_name.push(format!(".tmp.{}.{sequence}", std::process::id()));
    let temp = parent.join(temp_name);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temp)?;
        file.write_all(content)?;
        file.sync_all()?;
        fs::set_permissions(&temp, Permissions::from_mode(mode))?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
use super::TEMP_SEQUENCE;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
