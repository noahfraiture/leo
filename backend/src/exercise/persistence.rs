use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use serde::{Serialize, de::DeserializeOwned};

use super::{Error, Result};

/// Create a direct directory, rejecting a symlink in its place.
pub fn ensure_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => require_directory(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            ensure_directory(parent)?;
            match fs::create_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            require_directory(path)?;
            File::open(parent)?.sync_all()?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub fn require_directory(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err(Error::InvalidStorageEntry);
    }
    Ok(())
}

pub fn read_bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(Error::InvalidStorageEntry);
    }
    if metadata.len() > limit {
        return Err(Error::ImageTooLarge);
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::ImageTooLarge);
    }
    Ok(bytes)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&read_bytes(
        path,
        16 * 1024 * 1024,
    )?)?)
}

pub fn write_json(path: &Path, value: &impl Serialize, replace: bool) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_bytes(path, &bytes, replace)
}

/// Publish complete bytes, synchronizing both the file and its containing directory.
pub fn write_bytes(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    let parent = path.parent().ok_or(Error::InvalidStorageEntry)?;
    ensure_directory(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    if replace {
        temporary.persist(path).map_err(|error| error.error)?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
    }
    File::open(parent)?.sync_all()?;
    Ok(())
}

/// Serialize writers across store instances; dropping the returned file releases the lock.
pub fn lock(directory: &Path) -> Result<File> {
    let path = directory.join(".lock");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => return Err(Error::InvalidStorageEntry),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock()?;
    Ok(file)
}
