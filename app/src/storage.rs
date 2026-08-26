use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug)]
pub(crate) struct PersistError;

impl std::fmt::Display for PersistError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("private file persist failed")
    }
}

impl std::error::Error for PersistError {}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), PersistError> {
    let tmp = path.with_extension("json.tmp");
    let result = (|| {
        let mut file = File::create(&tmp)?;
        restrict_permissions(&file)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        if let Ok(file) = File::open(path) {
            restrict_permissions(&file)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|_: io::Error| PersistError)
}

fn restrict_permissions(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o600);
        file.set_permissions(permissions)?;
    }
    Ok(())
}
