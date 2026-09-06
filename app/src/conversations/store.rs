use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::id::ConversationId;

const CATALOGUE_VERSION: u32 = 1;
const CATALOGUE_FILE: &str = "catalogue.json";
const MAXIMUM_CATALOGUE_BYTES: usize = 256 * 1024;
pub(crate) const MAXIMUM_CONVERSATIONS: usize = 128;
pub(crate) const MAXIMUM_TITLE_BYTES: usize = 120;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationRecord {
    pub(crate) id: ConversationId,
    pub(crate) revision: u32,
    pub(crate) title: String,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConversationError {
    Random,
    Persist,
    Corrupt,
    Full,
    Missing,
    Conflict,
    Revision,
    Title,
}

impl ConversationError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Random => "Power Plant could not create a conversation identifier. Try again.",
            Self::Persist => "Power Plant could not store the conversation. Try again.",
            Self::Corrupt => "The conversation catalogue is unreadable.",
            Self::Full => {
                "The conversation catalogue is full. Delete a conversation before you create another."
            }
            Self::Missing => "That conversation is not in the catalogue.",
            Self::Conflict => "That conversation changed in another tab. Reload it.",
            Self::Revision => "Power Plant cannot update this conversation again.",
            Self::Title => "Enter a title of 1 to 120 bytes without control characters.",
        }
    }
}

impl std::fmt::Display for ConversationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ConversationError {}

pub(crate) struct ConversationStore {
    path: Option<PathBuf>,
    inner: Mutex<BTreeMap<ConversationId, ConversationRecord>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct CatalogueFile {
    version: u32,
    conversations: Vec<ConversationFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ConversationFile {
    id: String,
    revision: u32,
    title: String,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl ConversationStore {
    pub(crate) fn open(dir: PathBuf) -> Result<Self, ConversationError> {
        crate::storage::ensure_private_dir(&dir).map_err(|_| ConversationError::Persist)?;
        let path = crate::storage::confined_child(&dir, CATALOGUE_FILE)
            .map_err(|_| ConversationError::Persist)?;
        let conversations = load_path(&path)?;
        Ok(Self {
            path: Some(path),
            inner: Mutex::new(conversations),
        })
    }

    pub(crate) fn list(&self) -> Vec<ConversationRecord> {
        self.lock().values().cloned().collect()
    }

    pub(crate) fn get(&self, id: &ConversationId) -> Option<ConversationRecord> {
        self.lock().get(id).cloned()
    }

    pub(crate) fn create(&self, title: String) -> Result<ConversationRecord, ConversationError> {
        let title = normalise_title(&title)?;
        let mut conversations = self.lock();
        if conversations.len() >= MAXIMUM_CONVERSATIONS {
            return Err(ConversationError::Full);
        }
        let id = unused_identifier(&conversations)?;
        let now = now_ms();
        let record = ConversationRecord {
            id,
            revision: 1,
            title,
            created_at_ms: now,
            updated_at_ms: now,
        };
        conversations.insert(id, record.clone());
        if let Err(error) = persist(self.path.as_deref(), &conversations) {
            conversations.remove(&id);
            return Err(error);
        }
        Ok(record)
    }

    pub(crate) fn rename(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        title: String,
    ) -> Result<ConversationRecord, ConversationError> {
        let title = normalise_title(&title)?;
        let mut conversations = self.lock();
        let Some(current) = conversations.get(id).cloned() else {
            return Err(ConversationError::Missing);
        };
        if current.revision != expected_revision {
            return Err(ConversationError::Conflict);
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or(ConversationError::Revision)?;
        let record = ConversationRecord {
            id: current.id,
            revision,
            title,
            created_at_ms: current.created_at_ms,
            updated_at_ms: now_ms().max(current.updated_at_ms),
        };
        conversations.insert(*id, record.clone());
        if let Err(error) = persist(self.path.as_deref(), &conversations) {
            conversations.insert(*id, current);
            return Err(error);
        }
        Ok(record)
    }

    pub(crate) fn delete(
        &self,
        id: &ConversationId,
        expected_revision: u32,
    ) -> Result<(), ConversationError> {
        let mut conversations = self.lock();
        let Some(current) = conversations.get(id).cloned() else {
            return Err(ConversationError::Missing);
        };
        if current.revision != expected_revision {
            return Err(ConversationError::Conflict);
        }
        conversations.remove(id);
        if let Err(error) = persist(self.path.as_deref(), &conversations) {
            conversations.insert(*id, current);
            return Err(error);
        }
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<ConversationId, ConversationRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn unused_identifier(
    conversations: &BTreeMap<ConversationId, ConversationRecord>,
) -> Result<ConversationId, ConversationError> {
    for _ in 0..16 {
        let id = ConversationId::generate().map_err(|_| ConversationError::Random)?;
        if !conversations.contains_key(&id) {
            return Ok(id);
        }
    }
    Err(ConversationError::Random)
}

fn load_path(
    path: &Path,
) -> Result<BTreeMap<ConversationId, ConversationRecord>, ConversationError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(_) => return Err(ConversationError::Corrupt),
    }
    let bytes = crate::storage::read_private_bounded(path, MAXIMUM_CATALOGUE_BYTES)
        .map_err(|_| ConversationError::Corrupt)?;
    let file: CatalogueFile =
        serde_json::from_slice(&bytes).map_err(|_| ConversationError::Corrupt)?;
    state_from_file(file)
}

fn state_from_file(
    file: CatalogueFile,
) -> Result<BTreeMap<ConversationId, ConversationRecord>, ConversationError> {
    if file.version != CATALOGUE_VERSION || file.conversations.len() > MAXIMUM_CONVERSATIONS {
        return Err(ConversationError::Corrupt);
    }
    let mut conversations = BTreeMap::new();
    for file in file.conversations {
        let record = record_from_file(file)?;
        if conversations.insert(record.id, record).is_some() {
            return Err(ConversationError::Corrupt);
        }
    }
    Ok(conversations)
}

fn record_from_file(file: ConversationFile) -> Result<ConversationRecord, ConversationError> {
    let id = ConversationId::parse(&file.id).ok_or(ConversationError::Corrupt)?;
    if file.revision == 0 || file.updated_at_ms < file.created_at_ms {
        return Err(ConversationError::Corrupt);
    }
    let title = normalise_title(&file.title).map_err(|_| ConversationError::Corrupt)?;
    if title != file.title {
        return Err(ConversationError::Corrupt);
    }
    Ok(ConversationRecord {
        id,
        revision: file.revision,
        title,
        created_at_ms: file.created_at_ms,
        updated_at_ms: file.updated_at_ms,
    })
}

fn persist(
    path: Option<&Path>,
    conversations: &BTreeMap<ConversationId, ConversationRecord>,
) -> Result<(), ConversationError> {
    let Some(path) = path else {
        return Ok(());
    };
    let records = conversations.values().map(record_to_file).collect();
    let bytes = serde_json::to_vec_pretty(&CatalogueFile {
        version: CATALOGUE_VERSION,
        conversations: records,
    })
    .map_err(|_| ConversationError::Persist)?;
    if bytes.len() > MAXIMUM_CATALOGUE_BYTES {
        return Err(ConversationError::Full);
    }
    let dir = path.parent().ok_or(ConversationError::Persist)?;
    crate::storage::ensure_private_dir(dir).map_err(|_| ConversationError::Persist)?;
    crate::storage::write_private(path, &bytes).map_err(|_| ConversationError::Persist)
}

fn record_to_file(record: &ConversationRecord) -> ConversationFile {
    ConversationFile {
        id: record.id.as_hex(),
        revision: record.revision,
        title: record.title.clone(),
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
    }
}

fn normalise_title(raw: &str) -> Result<String, ConversationError> {
    let title = raw.trim();
    if title.is_empty() || title.len() > MAXIMUM_TITLE_BYTES || title.chars().any(char::is_control)
    {
        return Err(ConversationError::Title);
    }
    Ok(title.to_owned())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
