use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::id::ProjectId;
use super::record::{
    MAXIMUM_PROJECTS, ProjectError, ProjectFile, ProjectRecord, submitted_host_path,
};

const CATALOGUE_FILE_VERSION: u32 = 1;
const MAXIMUM_CATALOGUE_BYTES: usize = 512 * 1024;

pub(crate) struct ProjectStore {
    path: Option<PathBuf>,
    inner: Mutex<BTreeMap<ProjectId, ProjectRecord>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct CatalogueFile {
    file_version: u32,
    projects: Vec<ProjectFile>,
}

impl ProjectStore {
    pub(crate) fn open(path: PathBuf) -> Result<Self, ProjectError> {
        let dir = path.parent().ok_or(ProjectError::Persist)?;
        crate::storage::ensure_private_dir(dir).map_err(|_| ProjectError::Persist)?;
        let projects = load_path(&path)?;
        Ok(Self {
            path: Some(path),
            inner: Mutex::new(projects),
        })
    }

    pub(crate) fn list(&self) -> Vec<ProjectRecord> {
        self.lock().values().cloned().collect()
    }

    pub(crate) fn get(&self, id: &ProjectId) -> Option<ProjectRecord> {
        self.lock().get(id).cloned()
    }

    pub(crate) fn create(
        &self,
        name: String,
        host_path: PathBuf,
    ) -> Result<ProjectRecord, ProjectError> {
        let host_path = submitted_host_path(&host_path)?;
        let mut projects = self.lock();
        if projects.len() >= MAXIMUM_PROJECTS {
            return Err(ProjectError::Full);
        }
        let id = unused_identifier(&projects)?;
        let record = ProjectRecord::create(id, name, host_path, now_ms())?;
        if projects
            .values()
            .any(|item| item.host_path == record.host_path)
        {
            return Err(ProjectError::DuplicatePath);
        }
        projects.insert(record.id, record.clone());
        if let Err(error) = persist(self.path.as_deref(), &projects) {
            projects.remove(&record.id);
            return Err(error);
        }
        Ok(record)
    }

    pub(crate) fn update_name(
        &self,
        id: &ProjectId,
        expected_revision: u32,
        name: String,
    ) -> Result<ProjectRecord, ProjectError> {
        let mut projects = self.lock();
        let Some(current) = projects.get(id).cloned() else {
            return Err(ProjectError::Missing);
        };
        if current.revision != expected_revision {
            return Err(ProjectError::Conflict);
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or(ProjectError::Conflict)?;
        let record = ProjectRecord {
            id: current.id,
            revision,
            name: super::record::submitted_name(&name)?,
            host_path: current.host_path.clone(),
            created_at_ms: current.created_at_ms,
        };
        projects.insert(record.id, record.clone());
        if let Err(error) = persist(self.path.as_deref(), &projects) {
            projects.insert(current.id, current);
            return Err(error);
        }
        Ok(record)
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<ProjectId, ProjectRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn unused_identifier(
    projects: &BTreeMap<ProjectId, ProjectRecord>,
) -> Result<ProjectId, ProjectError> {
    for _ in 0..16 {
        let id = ProjectId::generate().map_err(|_| ProjectError::Random)?;
        if !projects.contains_key(&id) {
            return Ok(id);
        }
    }
    Err(ProjectError::Random)
}

fn load_path(path: &Path) -> Result<BTreeMap<ProjectId, ProjectRecord>, ProjectError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(_) => return Err(ProjectError::Corrupt),
    }
    let bytes = crate::storage::read_private_bounded(path, MAXIMUM_CATALOGUE_BYTES)
        .map_err(|_| ProjectError::Corrupt)?;
    let file: CatalogueFile = serde_json::from_slice(&bytes).map_err(|_| ProjectError::Corrupt)?;
    state_from_file(file)
}

fn state_from_file(
    file: CatalogueFile,
) -> Result<BTreeMap<ProjectId, ProjectRecord>, ProjectError> {
    if file.file_version != CATALOGUE_FILE_VERSION {
        return Err(ProjectError::Corrupt);
    }
    if file.projects.len() > MAXIMUM_PROJECTS {
        return Err(ProjectError::Corrupt);
    }
    let mut projects = BTreeMap::new();
    for item in file.projects {
        let record = ProjectRecord::from_file(item)?;
        if projects
            .values()
            .any(|item: &ProjectRecord| item.host_path == record.host_path)
        {
            return Err(ProjectError::Corrupt);
        }
        if projects.insert(record.id, record).is_some() {
            return Err(ProjectError::Corrupt);
        }
    }
    Ok(projects)
}

fn persist(
    path: Option<&Path>,
    projects: &BTreeMap<ProjectId, ProjectRecord>,
) -> Result<(), ProjectError> {
    let Some(path) = path else {
        return Ok(());
    };
    let mut records: Vec<ProjectFile> = projects.values().map(ProjectRecord::to_file).collect();
    records.sort_by(|left, right| left.id.cmp(&right.id));
    let file = CatalogueFile {
        file_version: CATALOGUE_FILE_VERSION,
        projects: records,
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|_| ProjectError::Persist)?;
    if bytes.len() > MAXIMUM_CATALOGUE_BYTES {
        return Err(ProjectError::Full);
    }
    let dir = path.parent().ok_or(ProjectError::Persist)?;
    crate::storage::ensure_private_dir(dir).map_err(|_| ProjectError::Persist)?;
    crate::storage::write_private(path, &bytes).map_err(|_| ProjectError::Persist)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
