use super::*;

pub(super) fn read_optional(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    let mut content = Vec::new();
    file.take(limit + 1).read_to_end(&mut content)?;
    if content.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    Ok(Some(content))
}

pub(super) fn write_backup(path: &Path, existing: Option<&[u8]>) -> io::Result<()> {
    let Some(existing) = existing else {
        return Ok(());
    };
    write_atomic(&backup_path(path), existing, existing_mode(path, 0o600))
}

pub(super) fn remove_backup(path: &Path) -> io::Result<()> {
    match fs::remove_file(backup_path(path)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
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
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
