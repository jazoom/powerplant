use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::id::AgentId;
use super::record::{
    AccessMode, AgentDraft, AgentError, AgentFile, AgentRecord, DEFAULT_AGENT_NAME,
    DEFAULT_PRIMARY_ALIAS, DirectoryGrant, MAXIMUM_AGENTS,
};
use super::tool_id::ToolId;

#[cfg(test)]
mod tests;

#[derive(Deserialize, Serialize)]
struct LegacyProjectFile {
    version: u32,
    path: Option<String>,
}

pub(crate) struct AgentStore {
    dir: Option<PathBuf>,
    inner: Mutex<BTreeMap<AgentId, AgentRecord>>,
}

impl AgentStore {
    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self {
            dir: None,
            inner: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn open(dir: PathBuf, legacy_project: &Path) -> Result<Self, AgentError> {
        fs::create_dir_all(&dir).map_err(|_| AgentError::Persist)?;
        let mut agents = load_dir(&dir)?;
        if agents.is_empty()
            && let Some(draft) = import_legacy(legacy_project)?
        {
            let record = persist_new(&dir, draft)?;
            agents.insert(record.id, record);
        }
        if !agents.is_empty() {
            remove_legacy(legacy_project)?;
        }
        Ok(Self {
            dir: Some(dir),
            inner: Mutex::new(agents),
        })
    }

    pub(crate) fn list(&self) -> Vec<AgentRecord> {
        self.lock().values().cloned().collect()
    }

    pub(crate) fn get(&self, id: &AgentId) -> Option<AgentRecord> {
        self.lock().get(id).cloned()
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.lock().len()
    }

    pub(crate) fn create(&self, draft: AgentDraft) -> Result<AgentRecord, AgentError> {
        let draft = draft.validate()?;
        let mut agents = self.lock();
        if agents.len() >= MAXIMUM_AGENTS {
            return Err(AgentError::Full);
        }
        let id = AgentId::generate().map_err(|_| AgentError::Random)?;
        let record = AgentRecord {
            id,
            revision: 1,
            name: draft.name,
            instructions: draft.instructions,
            tools: draft.tools,
            directories: draft.directories,
            primary_directory: draft.primary_directory,
        };
        persist(self.dir.as_deref(), &record)?;
        agents.insert(record.id, record.clone());
        Ok(record)
    }

    pub(crate) fn update(
        &self,
        id: &AgentId,
        draft: AgentDraft,
    ) -> Result<AgentRecord, AgentError> {
        let draft = draft.validate()?;
        let mut agents = self.lock();
        let Some(current) = agents.get(id).cloned() else {
            return Err(AgentError::Missing);
        };
        let revision = current
            .revision
            .checked_add(1)
            .ok_or(AgentError::Revision)?;
        let record = AgentRecord {
            id: current.id,
            revision,
            name: draft.name,
            instructions: draft.instructions,
            tools: draft.tools,
            directories: draft.directories,
            primary_directory: draft.primary_directory,
        };
        persist(self.dir.as_deref(), &record)?;
        agents.insert(record.id, record.clone());
        Ok(record)
    }

    pub(crate) fn delete(&self, id: &AgentId) -> Result<(), AgentError> {
        let mut agents = self.lock();
        if !agents.contains_key(id) {
            return Err(AgentError::Missing);
        }
        if let Some(dir) = &self.dir {
            remove_file(&dir.join(format!("{}.json", id.as_hex())))?;
        }
        agents.remove(id);
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<AgentId, AgentRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn load_dir(dir: &Path) -> Result<BTreeMap<AgentId, AgentRecord>, AgentError> {
    let mut agents = BTreeMap::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(agents),
        Err(_) => return Err(AgentError::Persist),
    };
    for entry in entries {
        let entry = entry.map_err(|_| AgentError::Persist)?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).map_err(|_| AgentError::Corrupt)?;
        let file: AgentFile = serde_json::from_slice(&bytes).map_err(|_| AgentError::Corrupt)?;
        let record = AgentRecord::from_file(file)?;
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or(AgentError::Corrupt)?;
        if stem != record.id.as_hex() {
            return Err(AgentError::Corrupt);
        }
        if agents.len() >= MAXIMUM_AGENTS || agents.insert(record.id, record).is_some() {
            return Err(AgentError::Corrupt);
        }
    }
    Ok(agents)
}

fn persist(dir: Option<&Path>, record: &AgentRecord) -> Result<(), AgentError> {
    let Some(dir) = dir else {
        return Ok(());
    };
    fs::create_dir_all(dir).map_err(|_| AgentError::Persist)?;
    let path = dir.join(format!("{}.json", record.id.as_hex()));
    let bytes = serde_json::to_vec_pretty(&record.to_file()).map_err(|_| AgentError::Persist)?;
    crate::storage::write_private(&path, &bytes).map_err(|_| AgentError::Persist)
}

fn persist_new(dir: &Path, draft: AgentDraft) -> Result<AgentRecord, AgentError> {
    let draft = draft.validate()?;
    let id = AgentId::generate().map_err(|_| AgentError::Random)?;
    let record = AgentRecord {
        id,
        revision: 1,
        name: draft.name,
        instructions: draft.instructions,
        tools: draft.tools,
        directories: draft.directories,
        primary_directory: draft.primary_directory,
    };
    persist(Some(dir), &record)?;
    Ok(record)
}

fn import_legacy(path: &Path) -> Result<Option<AgentDraft>, AgentError> {
    let Some(host_path) = load_legacy_project(path) else {
        return Ok(None);
    };
    let draft = AgentDraft {
        name: DEFAULT_AGENT_NAME.to_owned(),
        instructions: String::new(),
        tools: ToolId::ALL.to_vec(),
        directories: vec![DirectoryGrant {
            alias: DEFAULT_PRIMARY_ALIAS.to_owned(),
            host_path,
            access: AccessMode::ReadWrite,
        }],
        primary_directory: DEFAULT_PRIMARY_ALIAS.to_owned(),
    };
    match draft.validate() {
        Ok(draft) => Ok(Some(draft)),
        Err(
            AgentError::Path
            | AgentError::PathMissing
            | AgentError::NotADirectory
            | AgentError::PathAccess,
        ) => Ok(None),
        Err(error) => Err(error),
    }
}

fn load_legacy_project(path: &Path) -> Option<PathBuf> {
    let bytes = fs::read(path).ok()?;
    let file: LegacyProjectFile = serde_json::from_slice(&bytes).ok()?;
    if file.version != 1 {
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

fn remove_legacy(path: &Path) -> Result<(), AgentError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(AgentError::Persist),
    }
}

fn remove_file(path: &Path) -> Result<(), AgentError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(AgentError::Persist),
    }
}
