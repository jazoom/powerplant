use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::providers::ModelSelection;
use crate::sessions::JobId;

use super::id::ConversationId;

const CATALOGUE_VERSION: u32 = 1;
const CATALOGUE_FILE: &str = "catalogue.json";
const MAXIMUM_CATALOGUE_BYTES: usize = 1024 * 1024;
pub(crate) const MAXIMUM_CONVERSATIONS: usize = 128;
pub(crate) const MAXIMUM_TITLE_BYTES: usize = 120;
pub(crate) const MAXIMUM_MESSAGES: usize = 512;
pub(crate) const MAXIMUM_MESSAGE_BYTES: usize = 32 * 1024;
pub(crate) const MAXIMUM_REPLY_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationRecord {
    pub(crate) id: ConversationId,
    pub(crate) revision: u32,
    pub(crate) title: String,
    pub(crate) selection: Option<ModelSelection>,
    pub(crate) messages: Vec<ConversationMessage>,
    pub(crate) active_job: Option<JobId>,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MessageStatus {
    Complete,
    Pending,
    Interrupted,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationMessage {
    pub(crate) role: MessageRole,
    pub(crate) text: String,
    pub(crate) status: MessageStatus,
    pub(crate) request: Option<JobId>,
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
    Message,
    Active,
    Selection,
}

impl ConversationError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Random => "Power Plant could not create a conversation identifier. Try again.",
            Self::Persist => "Power Plant could not store the conversation. Try again.",
            Self::Corrupt => "The conversation catalogue is unreadable.",
            Self::Full => {
                "Delete an inactive conversation to free local history space. If this discussion reached its message limit, start another conversation."
            }
            Self::Missing => "That conversation is not in the catalogue.",
            Self::Conflict => "That conversation changed in another tab. Reload it.",
            Self::Revision => "Power Plant cannot update this conversation again.",
            Self::Title => "Enter a title of 1 to 120 bytes without control characters.",
            Self::Message => "Enter a message within the conversation limit.",
            Self::Active => "This conversation has an active request. Wait for it to finish.",
            Self::Selection => "Choose an available model before you send a message.",
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
    #[serde(deserialize_with = "crate::storage::required_option")]
    selection: Option<ModelSelection>,
    messages: Vec<MessageFile>,
    #[serde(deserialize_with = "crate::storage::required_option")]
    active_job: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct MessageFile {
    role: MessageRole,
    text: String,
    status: MessageStatus,
    #[serde(deserialize_with = "crate::storage::required_option")]
    request: Option<String>,
}

impl ConversationStore {
    pub(crate) fn open(dir: PathBuf) -> Result<Self, ConversationError> {
        crate::storage::ensure_private_dir(&dir).map_err(|_| ConversationError::Persist)?;
        let path = crate::storage::confined_child(&dir, CATALOGUE_FILE)
            .map_err(|_| ConversationError::Persist)?;
        let mut conversations = load_path(&path)?;
        if interrupt_recovered_requests(&mut conversations) {
            persist(Some(&path), &conversations)?;
        }
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
            selection: None,
            messages: Vec::new(),
            active_job: None,
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
        self.replace(id, expected_revision, |current| {
            current.title = title;
            Ok(())
        })
    }

    pub(crate) fn select_model(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        selection: ModelSelection,
    ) -> Result<ConversationRecord, ConversationError> {
        self.replace(id, expected_revision, |current| {
            if current.active_job.is_some() {
                return Err(ConversationError::Active);
            }
            current.selection = Some(selection);
            Ok(())
        })
    }

    pub(crate) fn begin_message(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        selection: ModelSelection,
        request: JobId,
        text: String,
    ) -> Result<ConversationRecord, ConversationError> {
        let text = normalise_message(&text)?;
        self.replace(id, expected_revision, |current| {
            if current.active_job.is_some() {
                return Err(ConversationError::Active);
            }
            if current.messages.len() > MAXIMUM_MESSAGES.saturating_sub(2) {
                return Err(ConversationError::Full);
            }
            current.selection = Some(selection);
            current.messages.push(ConversationMessage {
                role: MessageRole::User,
                text,
                status: MessageStatus::Complete,
                request: None,
            });
            current.messages.push(ConversationMessage {
                role: MessageRole::Assistant,
                text: String::new(),
                status: MessageStatus::Pending,
                request: Some(request),
            });
            current.active_job = Some(request);
            Ok(())
        })
    }

    pub(crate) fn append_output(
        &self,
        id: &ConversationId,
        request: JobId,
        text: String,
    ) -> Result<(), ConversationError> {
        if text.len() > MAXIMUM_REPLY_BYTES || text.contains('\0') {
            return Err(ConversationError::Message);
        }
        self.replace(id, 0, |current| {
            let message = active_assistant(current, request)?;
            message.text = text;
            Ok(())
        })
        .map(|_| ())
    }

    pub(crate) fn settle_message(
        &self,
        id: &ConversationId,
        request: JobId,
        text: String,
        status: MessageStatus,
    ) -> Result<(), ConversationError> {
        if !matches!(
            status,
            MessageStatus::Complete | MessageStatus::Interrupted | MessageStatus::Failed
        ) || text.len() > MAXIMUM_REPLY_BYTES
            || text.contains('\0')
        {
            return Err(ConversationError::Message);
        }
        self.replace(id, 0, |current| {
            let message = active_assistant(current, request)?;
            message.text = text;
            message.status = status;
            current.active_job = None;
            Ok(())
        })
        .map(|_| ())
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
        if current.active_job.is_some() {
            return Err(ConversationError::Active);
        }
        conversations.remove(id);
        if let Err(error) = persist(self.path.as_deref(), &conversations) {
            conversations.insert(*id, current);
            return Err(error);
        }
        Ok(())
    }

    fn replace(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        edit: impl FnOnce(&mut ConversationRecord) -> Result<(), ConversationError>,
    ) -> Result<ConversationRecord, ConversationError> {
        let mut conversations = self.lock();
        let Some(current) = conversations.get(id).cloned() else {
            return Err(ConversationError::Missing);
        };
        if expected_revision != 0 && current.revision != expected_revision {
            return Err(ConversationError::Conflict);
        }
        let mut updated = current.clone();
        edit(&mut updated)?;
        updated.revision = current
            .revision
            .checked_add(1)
            .ok_or(ConversationError::Revision)?;
        updated.updated_at_ms = now_ms().max(current.updated_at_ms);
        conversations.insert(*id, updated.clone());
        if let Err(error) = persist(self.path.as_deref(), &conversations) {
            conversations.insert(*id, current);
            return Err(error);
        }
        Ok(updated)
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<ConversationId, ConversationRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn active_assistant(
    record: &mut ConversationRecord,
    request: JobId,
) -> Result<&mut ConversationMessage, ConversationError> {
    if record.active_job != Some(request) {
        return Err(ConversationError::Conflict);
    }
    record
        .messages
        .iter_mut()
        .rev()
        .find(|message| {
            message.request == Some(request) && message.status == MessageStatus::Pending
        })
        .ok_or(ConversationError::Conflict)
}

fn interrupt_recovered_requests(
    conversations: &mut BTreeMap<ConversationId, ConversationRecord>,
) -> bool {
    let mut changed = false;
    for record in conversations.values_mut() {
        let Some(request) = record.active_job else {
            continue;
        };
        if let Some(message) = record
            .messages
            .iter_mut()
            .rev()
            .find(|message| message.request == Some(request))
        {
            message.status = MessageStatus::Interrupted;
        }
        record.active_job = None;
        record.revision = record.revision.saturating_add(1);
        record.updated_at_ms = now_ms().max(record.updated_at_ms);
        changed = true;
    }
    changed
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
    if file.revision == 0
        || file.updated_at_ms < file.created_at_ms
        || file.messages.len() > MAXIMUM_MESSAGES
    {
        return Err(ConversationError::Corrupt);
    }
    let title = normalise_title(&file.title).map_err(|_| ConversationError::Corrupt)?;
    if title != file.title {
        return Err(ConversationError::Corrupt);
    }
    let selection = match file.selection {
        Some(selection)
            if ModelSelection::new(
                selection.provider,
                selection.model.clone(),
                selection.thinking.clone(),
            )
            .as_ref()
                == Some(&selection) =>
        {
            Some(selection)
        }
        None => None,
        Some(_) => return Err(ConversationError::Corrupt),
    };
    let messages: Result<Vec<_>, _> = file.messages.into_iter().map(message_from_file).collect();
    let messages = messages?;
    let active_job = file.active_job.as_deref().and_then(JobId::parse);
    if file.active_job.is_some() && active_job.is_none() {
        return Err(ConversationError::Corrupt);
    }
    let pending: Vec<_> = messages
        .iter()
        .filter(|message| message.status == MessageStatus::Pending)
        .collect();
    if match active_job {
        Some(request) => {
            pending.len() != 1
                || pending[0].request != Some(request)
                || messages.last() != pending.first().copied()
        }
        None => !pending.is_empty(),
    } {
        return Err(ConversationError::Corrupt);
    }
    Ok(ConversationRecord {
        id,
        revision: file.revision,
        title,
        selection,
        messages,
        active_job,
        created_at_ms: file.created_at_ms,
        updated_at_ms: file.updated_at_ms,
    })
}

fn message_from_file(file: MessageFile) -> Result<ConversationMessage, ConversationError> {
    let limit = match file.role {
        MessageRole::User => MAXIMUM_MESSAGE_BYTES,
        MessageRole::Assistant => MAXIMUM_REPLY_BYTES,
    };
    if file.text.len() > limit || file.text.contains('\0') {
        return Err(ConversationError::Corrupt);
    }
    let request = file.request.as_deref().and_then(JobId::parse);
    if file.request.is_some() != request.is_some()
        || matches!(file.role, MessageRole::User) != request.is_none()
        || (file.role == MessageRole::User
            && (file.status != MessageStatus::Complete
                || normalise_message(&file.text).as_ref() != Ok(&file.text)))
    {
        return Err(ConversationError::Corrupt);
    }
    Ok(ConversationMessage {
        role: file.role,
        text: file.text,
        status: file.status,
        request,
    })
}

fn persist(
    path: Option<&Path>,
    conversations: &BTreeMap<ConversationId, ConversationRecord>,
) -> Result<(), ConversationError> {
    let records = conversations.values().map(record_to_file).collect();
    let bytes = serde_json::to_vec_pretty(&CatalogueFile {
        version: CATALOGUE_VERSION,
        conversations: records,
    })
    .map_err(|_| ConversationError::Persist)?;
    // Reserve worst-case JSON expansion and terminal status space before dispatch.
    let reserved: usize = conversations
        .values()
        .filter(|record| record.active_job.is_some())
        .map(|record| {
            let used = record
                .messages
                .last()
                .map_or(0, |message| message.text.len());
            6 * MAXIMUM_REPLY_BYTES.saturating_sub(used) + 64
        })
        .sum();
    if bytes.len().saturating_add(reserved) > MAXIMUM_CATALOGUE_BYTES {
        return Err(ConversationError::Full);
    }
    let Some(path) = path else {
        return Ok(());
    };
    let dir = path.parent().ok_or(ConversationError::Persist)?;
    crate::storage::ensure_private_dir(dir).map_err(|_| ConversationError::Persist)?;
    crate::storage::write_private(path, &bytes).map_err(|_| ConversationError::Persist)
}

fn record_to_file(record: &ConversationRecord) -> ConversationFile {
    ConversationFile {
        id: record.id.as_hex(),
        revision: record.revision,
        title: record.title.clone(),
        selection: record.selection.clone(),
        messages: record
            .messages
            .iter()
            .map(|message| MessageFile {
                role: message.role,
                text: message.text.clone(),
                status: message.status,
                request: message.request.map(|request| request.as_hex()),
            })
            .collect(),
        active_job: record.active_job.map(|request| request.as_hex()),
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

fn normalise_message(raw: &str) -> Result<String, ConversationError> {
    let text = raw.trim();
    if text.is_empty()
        || text.len() > MAXIMUM_MESSAGE_BYTES
        || text
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(ConversationError::Message);
    }
    Ok(text.to_owned())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
