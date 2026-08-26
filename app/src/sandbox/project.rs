use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::SandboxError;

pub(super) const GUEST_PROJECT: &str = "/project";
const PROJECT_FILE_VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
struct ProjectFile {
    version: u32,
    path: Option<String>,
}

pub(super) fn load(path: &Path) -> Option<PathBuf> {
    let bytes = fs::read(path).ok()?;
    let file: ProjectFile = serde_json::from_slice(&bytes).ok()?;
    if file.version != PROJECT_FILE_VERSION {
        return None;
    }
    let raw = file.path.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let stored = PathBuf::from(raw);
    if !stored.is_absolute() {
        return None;
    }
    Some(stored)
}

pub(super) fn persist(path: Option<&Path>, project: Option<&Path>) -> Result<(), SandboxError> {
    let Some(path) = path else {
        return Ok(());
    };
    if project.is_none() {
        return remove_file(path);
    }
    let file = ProjectFile {
        version: PROJECT_FILE_VERSION,
        path: project.map(|item| item.to_string_lossy().into_owned()),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|_| SandboxError::ProjectStore)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| SandboxError::ProjectStore)?;
    }
    crate::vault::write_private(path, &bytes).map_err(|_| SandboxError::ProjectStore)
}

pub(super) fn resolve_dir(path: &Path) -> Result<PathBuf, SandboxError> {
    let metadata = fs::metadata(path).map_err(map_fs_error)?;
    if !metadata.is_dir() {
        return Err(SandboxError::NotADirectory);
    }
    fs::canonicalize(path).map_err(map_fs_error)
}

pub(super) fn mounted_bind(config: &microsandbox::SandboxConfig) -> Option<PathBuf> {
    config.spec.mounts.iter().find_map(|mount| match mount {
        microsandbox::sandbox::VolumeMount::Bind { host, guest, .. } if guest == GUEST_PROJECT => {
            Some(host.clone())
        }
        _ => None,
    })
}

fn map_fs_error(error: io::Error) -> SandboxError {
    match error.kind() {
        io::ErrorKind::NotFound => SandboxError::DirectoryMissing,
        io::ErrorKind::NotADirectory => SandboxError::NotADirectory,
        _ => SandboxError::DirectoryAccess,
    }
}

fn remove_file(path: &Path) -> Result<(), SandboxError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(SandboxError::ProjectStore),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{GUEST_PROJECT, mounted_bind, resolve_dir};
    use crate::sandbox::SandboxError;

    fn config_with(mounts: Vec<microsandbox::sandbox::VolumeMount>) -> microsandbox::SandboxConfig {
        let mut config = microsandbox::SandboxConfig::default();
        config.spec.mounts = mounts;
        config
    }

    fn bind(guest: &str, host: &str) -> microsandbox::sandbox::VolumeMount {
        microsandbox::sandbox::MountBuilder::new(guest)
            .bind(host)
            .build()
            .expect("bind mount")
    }

    #[test]
    fn mounted_bind_reads_the_sdk_project_mount() {
        let config = config_with(vec![bind(GUEST_PROJECT, "/home/dev/app")]);
        assert_eq!(
            mounted_bind(&config).as_deref(),
            Some(Path::new("/home/dev/app"))
        );
    }

    #[test]
    fn mounted_bind_ignores_non_project_mounts() {
        let tmp = microsandbox::sandbox::MountBuilder::new("/tmp")
            .tmpfs()
            .build()
            .expect("tmpfs");
        let config = config_with(vec![tmp, bind("/elsewhere", "/secret")]);
        assert!(mounted_bind(&config).is_none());
    }

    #[test]
    fn resolve_dir_rejects_a_missing_path() {
        assert!(matches!(
            resolve_dir(Path::new("/no/such/powerplant-project")),
            Err(SandboxError::DirectoryMissing)
        ));
    }

    #[test]
    fn resolve_dir_rejects_a_file() {
        let dir = tempfile::tempdir().expect("dir");
        let file = dir.path().join("file");
        std::fs::write(&file, b"x").expect("file");
        assert!(matches!(
            resolve_dir(&file),
            Err(SandboxError::NotADirectory)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_dir_reports_access_failures() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("dir");
        let parent = dir.path().join("locked");
        let path = parent.join("project");
        std::fs::create_dir_all(&path).expect("subdir");
        let original = std::fs::metadata(&parent).expect("metadata").permissions();
        let mut locked = original.clone();
        locked.set_mode(0o000);
        std::fs::set_permissions(&parent, locked).expect("lock");
        let result = resolve_dir(&path);
        std::fs::set_permissions(&parent, original).expect("restore");
        assert!(matches!(result, Err(SandboxError::DirectoryAccess)));
    }
}
