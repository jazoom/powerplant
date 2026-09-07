use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use rand::{rand_core::TryRng, rngs::SysRng};

use crate::{execution::ExecutionSettings, sessions::SessionId};

use super::record::{
    MAXIMUM_PRESETS, PresetError, PresetFile, PresetId, PresetProvenance, PresetRecord,
    normalise_name,
};

#[derive(Clone)]
pub(crate) struct PresetPreview {
    pub(crate) token: String,
    pub(crate) record: PresetRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PresetDestination {
    Conversation(crate::conversations::ConversationId, u32),
    Draft(String),
}

struct PreviewRecord {
    session: SessionId,
    destination: PresetDestination,
    expires: std::time::Instant,
    preset: PresetRecord,
}

const MAXIMUM_PREVIEWS: usize = 256;
const PREVIEW_LIFETIME: std::time::Duration = std::time::Duration::from_secs(30 * 60);

pub(crate) struct PresetStore {
    dir: Option<PathBuf>,
    inner: Mutex<BTreeMap<PresetId, PresetRecord>>,
    previews: Mutex<BTreeMap<String, PreviewRecord>>,
    applied_drafts: Mutex<BTreeMap<String, PreviewRecord>>,
}

impl PresetStore {
    pub(crate) fn open(dir: PathBuf) -> Result<Self, PresetError> {
        crate::storage::ensure_private_dir(&dir).map_err(|_| PresetError::Persist)?;
        Ok(Self {
            inner: Mutex::new(load_dir(&dir)?),
            dir: Some(dir),
            previews: Mutex::new(BTreeMap::new()),
            applied_drafts: Mutex::new(BTreeMap::new()),
        })
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self {
            dir: None,
            inner: Mutex::new(BTreeMap::new()),
            previews: Mutex::new(BTreeMap::new()),
            applied_drafts: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn list(&self) -> Vec<PresetRecord> {
        self.lock().values().cloned().collect()
    }

    pub(crate) fn get(&self, id: &PresetId) -> Option<PresetRecord> {
        self.lock().get(id).cloned()
    }

    pub(crate) fn create(
        &self,
        name: &str,
        settings: ExecutionSettings,
        provenance: PresetProvenance,
    ) -> Result<PresetRecord, PresetError> {
        let name = normalise_name(name)?;
        let mut records = self.lock();
        if records.len() >= MAXIMUM_PRESETS {
            return Err(PresetError::Full);
        }
        let id = PresetId::generate()?;
        let record = PresetRecord {
            id,
            revision: 1,
            name,
            settings,
            provenance,
            created_at_ms: now_ms(),
        };
        persist(self.dir.as_deref(), &record)?;
        records.insert(id, record.clone());
        Ok(record)
    }

    pub(crate) fn update(
        &self,
        id: PresetId,
        revision: u32,
        name: &str,
        settings: ExecutionSettings,
    ) -> Result<PresetRecord, PresetError> {
        let name = normalise_name(name)?;
        let mut records = self.lock();
        let current = records.get(&id).ok_or(PresetError::Missing)?;
        if current.revision != revision {
            return Err(PresetError::Stale);
        }
        let mut record = current.clone();
        record.revision = revision.checked_add(1).ok_or(PresetError::Stale)?;
        record.name = name;
        record.settings = settings;
        persist(self.dir.as_deref(), &record)?;
        records.insert(id, record.clone());
        Ok(record)
    }

    pub(crate) fn delete(&self, id: PresetId, revision: u32) -> Result<(), PresetError> {
        let mut records = self.lock();
        let record = records.get(&id).ok_or(PresetError::Missing)?;
        if record.revision != revision {
            return Err(PresetError::Stale);
        }
        if let Some(dir) = &self.dir {
            let path = crate::storage::confined_child(dir, &format!("{id}.json"))
                .map_err(|_| PresetError::Persist)?;
            fs::remove_file(path).map_err(|_| PresetError::Persist)?;
        }
        records.remove(&id);
        Ok(())
    }

    pub(crate) fn preview(
        &self,
        session: SessionId,
        id: PresetId,
        destination: PresetDestination,
    ) -> Result<PresetPreview, PresetError> {
        let record = self.get(&id).ok_or(PresetError::Missing)?;
        let token = preview_token()?;
        let mut previews = self
            .previews
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        previews.retain(|_, preview| preview.expires > std::time::Instant::now());
        if previews.len() >= MAXIMUM_PREVIEWS {
            return Err(PresetError::PreviewFull);
        }
        previews.insert(
            token.clone(),
            PreviewRecord {
                session,
                destination,
                expires: std::time::Instant::now() + PREVIEW_LIFETIME,
                preset: record.clone(),
            },
        );
        Ok(PresetPreview { token, record })
    }

    pub(crate) fn consume_preview(
        &self,
        session: SessionId,
        token: &str,
        destination: &PresetDestination,
    ) -> Result<PresetRecord, PresetError> {
        let preview = self
            .previews
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(token)
            .ok_or(PresetError::Preview)?;
        if preview.session != session
            || &preview.destination != destination
            || preview.expires <= std::time::Instant::now()
        {
            return Err(PresetError::Preview);
        }
        Ok(preview.preset)
    }

    pub(crate) fn apply_draft_preview(
        &self,
        session: SessionId,
        token: &str,
        digest: String,
    ) -> Result<PresetRecord, PresetError> {
        let destination = PresetDestination::Draft(digest);
        let mut drafts = self
            .applied_drafts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        drafts.retain(|_, preview| preview.expires > std::time::Instant::now());
        if drafts.len() >= MAXIMUM_PREVIEWS {
            return Err(PresetError::PreviewFull);
        }
        let preset = self.consume_preview(session, token, &destination)?;
        drafts.insert(
            token.to_owned(),
            PreviewRecord {
                session,
                destination,
                expires: std::time::Instant::now() + PREVIEW_LIFETIME,
                preset: preset.clone(),
            },
        );
        Ok(preset)
    }

    pub(crate) fn applied_draft(&self, session: SessionId, token: &str) -> Option<PresetRecord> {
        self.applied_drafts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(token)
            .filter(|preview| {
                preview.session == session && preview.expires > std::time::Instant::now()
            })
            .map(|preview| preview.preset.clone())
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<PresetId, PresetRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn load_dir(dir: &Path) -> Result<BTreeMap<PresetId, PresetRecord>, PresetError> {
    let mut records = BTreeMap::new();
    let entries = fs::read_dir(dir).map_err(|_| PresetError::Persist)?;
    for entry in entries {
        let path = entry.map_err(|_| PresetError::Persist)?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let bytes = crate::storage::read_private_bounded(&path, 256 * 1024)
            .map_err(|_| PresetError::Corrupt)?;
        let file: PresetFile = serde_json::from_slice(&bytes).map_err(|_| PresetError::Corrupt)?;
        let record = PresetRecord::from_file(file)?;
        if path.file_stem().and_then(|stem| stem.to_str()) != Some(&record.id.as_hex())
            || records.len() >= MAXIMUM_PRESETS
            || records.insert(record.id, record).is_some()
        {
            return Err(PresetError::Corrupt);
        }
    }
    Ok(records)
}

fn persist(dir: Option<&Path>, record: &PresetRecord) -> Result<(), PresetError> {
    let Some(dir) = dir else {
        return Ok(());
    };
    let path = crate::storage::confined_child(dir, &format!("{}.json", record.id.as_hex()))
        .map_err(|_| PresetError::Persist)?;
    let bytes = serde_json::to_vec_pretty(&record.to_file()).map_err(|_| PresetError::Persist)?;
    crate::storage::write_private(&path, &bytes).map_err(|_| PresetError::Persist)
}

fn preview_token() -> Result<String, PresetError> {
    let mut bytes = [0_u8; 32];
    SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| PresetError::Random)?;
    Ok(crate::hex::encode(&bytes))
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
