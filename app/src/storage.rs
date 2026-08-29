use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rand::rand_core::TryRng;
use rand::rngs::SysRng;

const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Debug)]
pub(crate) struct PersistError;

impl std::fmt::Display for PersistError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("private file persist failed")
    }
}

impl std::error::Error for PersistError {}

pub(crate) fn ensure_private_dir(path: &Path) -> Result<(), PersistError> {
    fs::create_dir_all(path).map_err(|_| PersistError)?;
    restrict_dir_permissions(path).map_err(|_| PersistError)
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), PersistError> {
    let dir = path.parent().ok_or(PersistError)?;
    let (tmp, mut file) = create_unique_tmp(dir).map_err(|_| PersistError)?;
    let result: io::Result<()> = (|| {
        restrict_file_permissions(&file)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        sync_dir(dir)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = fs::remove_file(&tmp);
            Err(PersistError)
        }
    }
}

fn create_unique_tmp(dir: &Path) -> io::Result<(PathBuf, File)> {
    for _ in 0..16 {
        let mut bytes = [0u8; 8];
        SysRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| io::Error::other("system random source unavailable"))?;
        let mut name = String::from(".");
        for byte in bytes {
            name.push(HEX[(byte >> 4) as usize] as char);
            name.push(HEX[(byte & 0x0f) as usize] as char);
        }
        name.push_str(".tmp");
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "private temporary file collision",
    ))
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

fn restrict_file_permissions(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o600);
        file.set_permissions(permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = file;
    }
    Ok(())
}

fn restrict_dir_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
