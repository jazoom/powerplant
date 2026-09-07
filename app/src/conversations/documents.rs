use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::conversations::{ConversationId, ConversationRecord, MessageRole, MessageStatus};
use crate::workflows::artefacts::payload::encode_plan;
use crate::workflows::artefacts::{
    ArtefactHash, ObjectHash, TypedPayload, WorkflowArtefactRepository, artefact_hash_for,
    parse_typed_payload,
};
use crate::workflows::definition::ArtefactKind;

const CATALOGUE_VERSION: u32 = 1;
const CATALOGUE_FILE: &str = "catalogue.json";
const MAXIMUM_CATALOGUE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAXIMUM_DOCUMENTS: usize = 256;
pub(crate) const MAXIMUM_DOCUMENT_REVISIONS: usize = 32;
pub(crate) const MAXIMUM_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAXIMUM_DOCUMENT_TITLE_BYTES: usize = 120;
// The page includes both escaped source and rendered content within one Hypergraft envelope.
pub(crate) const MAXIMUM_DOCUMENT_CONTENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct DocumentId([u8; 16]);

impl DocumentId {
    pub(crate) fn generate() -> Result<Self, DocumentIdError> {
        use rand::rand_core::TryRng;
        use rand::rngs::SysRng;

        let mut bytes = [0_u8; 16];
        SysRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| DocumentIdError::RandomUnavailable)?;
        Ok(Self(bytes))
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        crate::hex::decode(value).map(Self)
    }

    pub(crate) fn as_hex(&self) -> String {
        crate::hex::encode(&self.0)
    }
}

impl std::fmt::Display for DocumentId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.as_hex())
    }
}

impl std::fmt::Debug for DocumentId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DocumentId(")?;
        formatter.write_str(&self.as_hex())?;
        formatter.write_str(")")
    }
}

#[derive(Debug)]
pub(crate) enum DocumentIdError {
    RandomUnavailable,
}

impl std::fmt::Display for DocumentIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("system random source unavailable")
    }
}

impl std::error::Error for DocumentIdError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DocumentKind {
    Plan,
    TaskList,
}

impl DocumentKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::TaskList => "task list",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlanRevisionReference {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: u32,
    pub(crate) content_hash: ObjectHash,
    pub(crate) object_hash: ObjectHash,
    pub(crate) artefact_hash: ArtefactHash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PlanSource {
    ConversationMessage {
        conversation_id: ConversationId,
        message_index: u32,
        source_hash: ObjectHash,
    },
    SubmittedText {
        conversation_id: ConversationId,
        source_hash: ObjectHash,
    },
    DirectoryFile {
        conversation_id: ConversationId,
        directory_id: crate::execution::DirectoryGrantId,
        path: String,
        source_hash: ObjectHash,
    },
    Correction {
        previous: PlanRevisionReference,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlanRevision {
    pub(crate) revision: u32,
    pub(crate) content_hash: ObjectHash,
    pub(crate) object_hash: ObjectHash,
    pub(crate) artefact_hash: ArtefactHash,
    pub(crate) content_bytes: u64,
    pub(crate) source: PlanSource,
    pub(crate) created_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlanDocument {
    pub(crate) id: DocumentId,
    pub(crate) kind: DocumentKind,
    pub(crate) title: String,
    pub(crate) associated_conversation: Option<ConversationId>,
    pub(crate) revisions: Vec<PlanRevision>,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
}

impl PlanDocument {
    pub(crate) fn current_revision(&self) -> u32 {
        self.revisions
            .last()
            .map_or(0, |revision| revision.revision)
    }

    pub(crate) fn current(&self) -> &PlanRevision {
        self.revisions
            .last()
            .expect("a plan document has at least one revision")
    }

    pub(crate) fn revision(&self, revision: u32) -> Option<&PlanRevision> {
        self.revisions.iter().find(|item| item.revision == revision)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentError {
    Random,
    Persist,
    Corrupt,
    Missing,
    Conflict,
    Revision,
    Full,
    Title,
    Content,
    TaskList,
    Credential,
    Source,
    Active,
}

impl DocumentError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Random => "Power Plant cannot create a plan identifier.",
            Self::Persist => "Power Plant cannot store that plan.",
            Self::Corrupt => "The plan document catalogue is unreadable.",
            Self::Missing => "That plan document is not available.",
            Self::Conflict => "That plan changed in another tab. Reload it.",
            Self::Revision => "Power Plant cannot create another plan revision.",
            Self::Full => {
                "The local plan store or revision limit is full, or the plan exceeds 65536 bytes."
            }
            Self::Title => "Enter a plan title of 1 to 120 bytes without control characters.",
            Self::Content => "Enter plan text with content within the plan limit.",
            Self::TaskList => {
                "Use a heading and at most 256 top-level checkbox tasks within 64 KiB. Put literal checkbox examples inside code fences."
            }
            Self::Credential => "Do not save provider credentials in a plan.",
            Self::Source => "Select a completed assistant message as the plan source.",
            Self::Active => "This conversation has an active request. Wait for it to finish.",
        }
    }
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for DocumentError {}

pub(crate) struct PlanDocumentStore {
    path: Option<PathBuf>,
    content: Arc<WorkflowArtefactRepository>,
    inner: Mutex<BTreeMap<DocumentId, PlanDocument>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct CatalogueFile {
    version: u32,
    documents: Vec<DocumentFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct DocumentFile {
    id: String,
    kind: DocumentKind,
    title: String,
    #[serde(deserialize_with = "crate::storage::required_option")]
    associated_conversation: Option<String>,
    revisions: Vec<RevisionFile>,
    created_at_ms: u64,
    updated_at_ms: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RevisionFile {
    revision: u32,
    content_hash: String,
    object_hash: String,
    artefact_hash: String,
    content_bytes: u64,
    source: SourceFile,
    created_at_ms: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "source", rename_all = "kebab-case")]
enum SourceFile {
    ConversationMessage {
        conversation_id: String,
        message_index: u32,
        source_hash: String,
    },
    SubmittedText {
        conversation_id: String,
        source_hash: String,
    },
    DirectoryFile {
        conversation_id: String,
        directory_id: String,
        path: String,
        source_hash: String,
    },
    Correction {
        previous: RevisionReferenceFile,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RevisionReferenceFile {
    document_id: String,
    revision: u32,
    content_hash: String,
    object_hash: String,
    artefact_hash: String,
}

impl PlanDocumentStore {
    pub(crate) fn open(
        dir: PathBuf,
        content: Arc<WorkflowArtefactRepository>,
    ) -> Result<Self, DocumentError> {
        crate::storage::ensure_private_dir(&dir).map_err(|_| DocumentError::Persist)?;
        let path = crate::storage::confined_child(&dir, CATALOGUE_FILE)
            .map_err(|_| DocumentError::Persist)?;
        let documents = load_path(&path, &content)?;
        Ok(Self {
            path: Some(path),
            content,
            inner: Mutex::new(documents),
        })
    }

    pub(crate) fn list_for_conversation(
        &self,
        conversation_id: ConversationId,
    ) -> Vec<PlanDocument> {
        let mut documents: Vec<_> = self
            .lock()
            .values()
            .filter(|document| document.associated_conversation == Some(conversation_id))
            .cloned()
            .collect();
        documents.sort_by(|left, right| left.title.cmp(&right.title));
        documents
    }

    pub(crate) fn get(&self, id: &DocumentId) -> Option<PlanDocument> {
        self.lock().get(id).cloned()
    }

    pub(crate) fn content(
        &self,
        document: &PlanDocument,
        revision: u32,
    ) -> Result<String, DocumentError> {
        let revision = document.revision(revision).ok_or(DocumentError::Missing)?;
        let text = read_revision(&self.content, revision)?;
        if document.kind == DocumentKind::TaskList {
            crate::workflows::task_list::parse(&text).map_err(|_| DocumentError::Corrupt)?;
        }
        Ok(text)
    }

    pub(crate) fn create_from_message(
        &self,
        conversation: &ConversationRecord,
        message_index: usize,
        title: String,
        secret: Option<&str>,
    ) -> Result<PlanDocument, DocumentError> {
        let Some(message) = conversation.messages.get(message_index) else {
            return Err(DocumentError::Source);
        };
        if message.role != MessageRole::Assistant
            || message.status != MessageStatus::Complete
            || message.text.trim().is_empty()
        {
            return Err(DocumentError::Source);
        }
        let source = PlanSource::ConversationMessage {
            conversation_id: conversation.id,
            message_index: u32::try_from(message_index).map_err(|_| DocumentError::Source)?,
            source_hash: ObjectHash::of(message.text.as_bytes()),
        };
        self.create(
            DocumentKind::Plan,
            conversation.id,
            title,
            &message.text,
            source,
            secret,
        )
    }

    pub(crate) fn create_task_list_from_message(
        &self,
        conversation: &ConversationRecord,
        message_index: usize,
        title: String,
        secret: Option<&str>,
    ) -> Result<PlanDocument, DocumentError> {
        let Some(message) = conversation.messages.get(message_index) else {
            return Err(DocumentError::Source);
        };
        if message.role != MessageRole::Assistant || message.status != MessageStatus::Complete {
            return Err(DocumentError::Source);
        }
        self.create_task_list(
            conversation.id,
            title,
            &message.text,
            PlanSource::ConversationMessage {
                conversation_id: conversation.id,
                message_index: u32::try_from(message_index).map_err(|_| DocumentError::Source)?,
                source_hash: ObjectHash::of(message.text.as_bytes()),
            },
            secret,
        )
    }

    pub(crate) fn create_task_list_from_text(
        &self,
        conversation_id: ConversationId,
        title: String,
        markdown: String,
        secret: Option<&str>,
    ) -> Result<PlanDocument, DocumentError> {
        self.create_task_list(
            conversation_id,
            title,
            &markdown,
            PlanSource::SubmittedText {
                conversation_id,
                source_hash: ObjectHash::of(markdown.as_bytes()),
            },
            secret,
        )
    }

    pub(crate) fn create_from_text(
        &self,
        conversation_id: ConversationId,
        title: String,
        markdown: String,
        secret: Option<&str>,
    ) -> Result<PlanDocument, DocumentError> {
        let source = PlanSource::SubmittedText {
            conversation_id,
            source_hash: ObjectHash::of(markdown.as_bytes()),
        };
        self.create(
            DocumentKind::Plan,
            conversation_id,
            title,
            &markdown,
            source,
            secret,
        )
    }

    pub(crate) fn create_task_list(
        &self,
        conversation_id: ConversationId,
        title: String,
        markdown: &str,
        source: PlanSource,
        secret: Option<&str>,
    ) -> Result<PlanDocument, DocumentError> {
        self.create(
            DocumentKind::TaskList,
            conversation_id,
            title,
            markdown,
            source,
            secret,
        )
    }

    pub(crate) fn revise(
        &self,
        id: &DocumentId,
        expected_revision: u32,
        title: String,
        markdown: String,
        secret: Option<&str>,
    ) -> Result<PlanDocument, DocumentError> {
        let title = normalise_title(&title)?;
        encode_document(&title, secret)?;
        let mut documents = self.lock();
        let current = documents.get(id).cloned().ok_or(DocumentError::Missing)?;
        let encoded = encode_document_kind(&markdown, secret, current.kind)?;
        if current.current_revision() != expected_revision {
            return Err(DocumentError::Conflict);
        }
        if current.kind == DocumentKind::TaskList {
            crate::workflows::task_list::parse(&markdown).map_err(|_| DocumentError::TaskList)?;
        }
        if current.revisions.len() >= MAXIMUM_DOCUMENT_REVISIONS {
            return Err(DocumentError::Full);
        }
        let total = total_bytes(&documents);
        if total
            .checked_add(encoded.content_bytes)
            .is_none_or(|bytes| bytes > MAXIMUM_DOCUMENT_BYTES as u64)
        {
            return Err(DocumentError::Full);
        }
        let revision_number = expected_revision
            .checked_add(1)
            .ok_or(DocumentError::Revision)?;
        let previous = current.current();
        self.content
            .publish(&encoded.bytes)
            .map_err(map_content_error)?;
        let now = now_ms().max(current.updated_at_ms);
        let mut updated = current.clone();
        updated.title = title;
        updated.updated_at_ms = now;
        updated.revisions.push(PlanRevision {
            revision: revision_number,
            content_hash: encoded.content_hash,
            object_hash: encoded.object_hash,
            artefact_hash: encoded.artefact_hash,
            content_bytes: encoded.content_bytes,
            source: PlanSource::Correction {
                previous: PlanRevisionReference {
                    document_id: current.id,
                    revision: previous.revision,
                    content_hash: previous.content_hash,
                    object_hash: previous.object_hash,
                    artefact_hash: previous.artefact_hash,
                },
            },
            created_at_ms: now,
        });
        documents.insert(*id, updated.clone());
        if let Err(error) = persist(self.path.as_deref(), &documents) {
            documents.insert(*id, current);
            return Err(error);
        }
        Ok(updated)
    }

    pub(crate) fn disassociate(
        &self,
        id: &DocumentId,
        expected_revision: u32,
        conversation_id: ConversationId,
    ) -> Result<(), DocumentError> {
        let mut documents = self.lock();
        let current = documents.get(id).cloned().ok_or(DocumentError::Missing)?;
        if current.current_revision() != expected_revision
            || current.associated_conversation != Some(conversation_id)
        {
            return Err(DocumentError::Conflict);
        }
        let mut updated = current.clone();
        updated.associated_conversation = None;
        documents.insert(*id, updated);
        if let Err(error) = persist(self.path.as_deref(), &documents) {
            documents.insert(*id, current);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn disassociate_conversation(
        &self,
        conversation_id: ConversationId,
    ) -> Result<(), DocumentError> {
        let mut documents = self.lock();
        let previous = documents.clone();
        let mut changed = false;
        for document in documents.values_mut() {
            if document.associated_conversation == Some(conversation_id) {
                document.associated_conversation = None;
                changed = true;
            }
        }
        if changed && let Err(error) = persist(self.path.as_deref(), &documents) {
            *documents = previous;
            return Err(error);
        }
        Ok(())
    }

    fn create(
        &self,
        kind: DocumentKind,
        conversation_id: ConversationId,
        title: String,
        markdown: &str,
        source: PlanSource,
        secret: Option<&str>,
    ) -> Result<PlanDocument, DocumentError> {
        let title = normalise_title(&title)?;
        encode_document(&title, secret)?;
        let encoded = encode_document_kind(markdown, secret, kind)?;
        if kind == DocumentKind::TaskList {
            crate::workflows::task_list::parse(markdown).map_err(|_| DocumentError::TaskList)?;
        }
        let mut documents = self.lock();
        if documents.len() >= MAXIMUM_DOCUMENTS {
            return Err(DocumentError::Full);
        }
        if total_bytes(&documents)
            .checked_add(encoded.content_bytes)
            .is_none_or(|bytes| bytes > MAXIMUM_DOCUMENT_BYTES as u64)
        {
            return Err(DocumentError::Full);
        }
        let id = unused_identifier(&documents)?;
        self.content
            .publish(&encoded.bytes)
            .map_err(map_content_error)?;
        let now = now_ms();
        let document = PlanDocument {
            id,
            kind,
            title,
            associated_conversation: Some(conversation_id),
            revisions: vec![PlanRevision {
                revision: 1,
                content_hash: encoded.content_hash,
                object_hash: encoded.object_hash,
                artefact_hash: encoded.artefact_hash,
                content_bytes: encoded.content_bytes,
                source,
                created_at_ms: now,
            }],
            created_at_ms: now,
            updated_at_ms: now,
        };
        documents.insert(id, document.clone());
        if let Err(error) = persist(self.path.as_deref(), &documents) {
            documents.remove(&id);
            return Err(error);
        }
        Ok(document)
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<DocumentId, PlanDocument>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct EncodedDocument {
    bytes: Vec<u8>,
    content_hash: ObjectHash,
    object_hash: ObjectHash,
    artefact_hash: ArtefactHash,
    content_bytes: u64,
}

fn encode_document(markdown: &str, secret: Option<&str>) -> Result<EncodedDocument, DocumentError> {
    encode_document_kind(markdown, secret, DocumentKind::Plan)
}

fn encode_document_kind(
    markdown: &str,
    secret: Option<&str>,
    kind: DocumentKind,
) -> Result<EncodedDocument, DocumentError> {
    if markdown.len() > MAXIMUM_DOCUMENT_CONTENT_BYTES {
        return Err(DocumentError::Full);
    }
    if markdown.trim().is_empty() {
        return Err(DocumentError::Content);
    }
    let encoding = match kind {
        DocumentKind::Plan => encode_plan(markdown, secret),
        DocumentKind::TaskList => {
            crate::workflows::artefacts::payload::encode_plan_verbatim(markdown, secret)
        }
    };
    let (bytes, object_hash, artefact_hash) = encoding.map_err(|error| match error {
        crate::workflows::artefacts::payload::PayloadError::Credential => DocumentError::Credential,
        crate::workflows::artefacts::payload::PayloadError::Bound => DocumentError::Full,
        crate::workflows::artefacts::payload::PayloadError::Text
        | crate::workflows::artefacts::payload::PayloadError::Format => DocumentError::Content,
        crate::workflows::artefacts::payload::PayloadError::Encoding
        | crate::workflows::artefacts::payload::PayloadError::DuplicateField
        | crate::workflows::artefacts::payload::PayloadError::Candidate => DocumentError::Content,
    })?;
    let TypedPayload::Plan(plan) =
        parse_typed_payload(ArtefactKind::Plan, &bytes).map_err(|_| DocumentError::Content)?
    else {
        return Err(DocumentError::Content);
    };
    Ok(EncodedDocument {
        content_bytes: u64::try_from(plan.markdown.len()).map_err(|_| DocumentError::Full)?,
        content_hash: ObjectHash::of(plan.markdown.as_bytes()),
        bytes,
        object_hash,
        artefact_hash,
    })
}

fn read_revision(
    content: &WorkflowArtefactRepository,
    revision: &PlanRevision,
) -> Result<String, DocumentError> {
    let bytes = content
        .get(&revision.object_hash)
        .map_err(map_content_error)?;
    if artefact_hash_for(ArtefactKind::Plan, 1, &bytes) != revision.artefact_hash {
        return Err(DocumentError::Corrupt);
    }
    let TypedPayload::Plan(plan) =
        parse_typed_payload(ArtefactKind::Plan, &bytes).map_err(|_| DocumentError::Corrupt)?
    else {
        return Err(DocumentError::Corrupt);
    };
    if ObjectHash::of(plan.markdown.as_bytes()) != revision.content_hash
        || plan.markdown.len() as u64 != revision.content_bytes
    {
        return Err(DocumentError::Corrupt);
    }
    Ok(plan.markdown)
}

fn load_path(
    path: &Path,
    content: &WorkflowArtefactRepository,
) -> Result<BTreeMap<DocumentId, PlanDocument>, DocumentError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(_) => return Err(DocumentError::Corrupt),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(DocumentError::Corrupt);
    }
    let bytes = crate::storage::read_private_bounded(path, MAXIMUM_CATALOGUE_BYTES)
        .map_err(|_| DocumentError::Corrupt)?;
    let file: CatalogueFile = serde_json::from_slice(&bytes).map_err(|_| DocumentError::Corrupt)?;
    if file.version != CATALOGUE_VERSION || file.documents.len() > MAXIMUM_DOCUMENTS {
        return Err(DocumentError::Corrupt);
    }
    let mut documents = BTreeMap::new();
    let mut total = 0_u64;
    for document in file.documents {
        let parsed = document_from_file(document)?;
        if parsed.revisions.len() > MAXIMUM_DOCUMENT_REVISIONS {
            return Err(DocumentError::Corrupt);
        }
        for (expected, revision) in parsed.revisions.iter().enumerate() {
            if revision.revision
                != u32::try_from(expected + 1).map_err(|_| DocumentError::Corrupt)?
                || revision.content_bytes > MAXIMUM_DOCUMENT_CONTENT_BYTES as u64
            {
                return Err(DocumentError::Corrupt);
            }
            let text = read_revision(content, revision)?;
            if parsed.kind == DocumentKind::TaskList {
                crate::workflows::task_list::parse(&text).map_err(|_| DocumentError::Corrupt)?;
            }
            total = total
                .checked_add(revision.content_bytes)
                .filter(|bytes| *bytes <= MAXIMUM_DOCUMENT_BYTES as u64)
                .ok_or(DocumentError::Corrupt)?;
        }
        if documents.insert(parsed.id, parsed).is_some() {
            return Err(DocumentError::Corrupt);
        }
    }
    validate_sources(&documents)?;
    Ok(documents)
}

fn validate_sources(documents: &BTreeMap<DocumentId, PlanDocument>) -> Result<(), DocumentError> {
    for document in documents.values() {
        for revision in &document.revisions {
            let PlanSource::Correction { previous } = &revision.source else {
                continue;
            };
            let Some(source_document) = documents.get(&previous.document_id) else {
                return Err(DocumentError::Corrupt);
            };
            let Some(source_revision) = source_document.revision(previous.revision) else {
                return Err(DocumentError::Corrupt);
            };
            if source_document.id != document.id
                || previous.revision >= revision.revision
                || source_revision.content_hash != previous.content_hash
                || source_revision.object_hash != previous.object_hash
                || source_revision.artefact_hash != previous.artefact_hash
            {
                return Err(DocumentError::Corrupt);
            }
        }
    }
    Ok(())
}

fn document_from_file(file: DocumentFile) -> Result<PlanDocument, DocumentError> {
    let id = DocumentId::parse(&file.id).ok_or(DocumentError::Corrupt)?;
    let title = normalise_title(&file.title).map_err(|_| DocumentError::Corrupt)?;
    let associated_conversation = file
        .associated_conversation
        .map(|value| ConversationId::parse(&value).ok_or(DocumentError::Corrupt))
        .transpose()?;
    if file.revisions.is_empty() {
        return Err(DocumentError::Corrupt);
    }
    let revisions = file
        .revisions
        .into_iter()
        .map(revision_from_file)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PlanDocument {
        id,
        kind: file.kind,
        title,
        associated_conversation,
        revisions,
        created_at_ms: file.created_at_ms,
        updated_at_ms: file.updated_at_ms,
    })
}

fn revision_from_file(file: RevisionFile) -> Result<PlanRevision, DocumentError> {
    Ok(PlanRevision {
        revision: file.revision,
        content_hash: ObjectHash::parse(&file.content_hash).ok_or(DocumentError::Corrupt)?,
        object_hash: ObjectHash::parse(&file.object_hash).ok_or(DocumentError::Corrupt)?,
        artefact_hash: ArtefactHash::parse(&file.artefact_hash).ok_or(DocumentError::Corrupt)?,
        content_bytes: file.content_bytes,
        source: source_from_file(file.source)?,
        created_at_ms: file.created_at_ms,
    })
}

fn source_from_file(file: SourceFile) -> Result<PlanSource, DocumentError> {
    match file {
        SourceFile::ConversationMessage {
            conversation_id,
            message_index,
            source_hash,
        } => Ok(PlanSource::ConversationMessage {
            conversation_id: ConversationId::parse(&conversation_id)
                .ok_or(DocumentError::Corrupt)?,
            message_index,
            source_hash: ObjectHash::parse(&source_hash).ok_or(DocumentError::Corrupt)?,
        }),
        SourceFile::SubmittedText {
            conversation_id,
            source_hash,
        } => Ok(PlanSource::SubmittedText {
            conversation_id: ConversationId::parse(&conversation_id)
                .ok_or(DocumentError::Corrupt)?,
            source_hash: ObjectHash::parse(&source_hash).ok_or(DocumentError::Corrupt)?,
        }),
        SourceFile::DirectoryFile {
            conversation_id,
            directory_id,
            path,
            source_hash,
        } => {
            if !crate::workflows::task_list::valid_project_path(&path) {
                return Err(DocumentError::Corrupt);
            }
            Ok(PlanSource::DirectoryFile {
                conversation_id: ConversationId::parse(&conversation_id)
                    .ok_or(DocumentError::Corrupt)?,
                directory_id: crate::execution::DirectoryGrantId::parse(&directory_id)
                    .ok_or(DocumentError::Corrupt)?,
                path,
                source_hash: ObjectHash::parse(&source_hash).ok_or(DocumentError::Corrupt)?,
            })
        }
        SourceFile::Correction { previous } => Ok(PlanSource::Correction {
            previous: reference_from_file(previous)?,
        }),
    }
}

fn reference_from_file(
    file: RevisionReferenceFile,
) -> Result<PlanRevisionReference, DocumentError> {
    Ok(PlanRevisionReference {
        document_id: DocumentId::parse(&file.document_id).ok_or(DocumentError::Corrupt)?,
        revision: file.revision,
        content_hash: ObjectHash::parse(&file.content_hash).ok_or(DocumentError::Corrupt)?,
        object_hash: ObjectHash::parse(&file.object_hash).ok_or(DocumentError::Corrupt)?,
        artefact_hash: ArtefactHash::parse(&file.artefact_hash).ok_or(DocumentError::Corrupt)?,
    })
}

fn persist(
    path: Option<&Path>,
    documents: &BTreeMap<DocumentId, PlanDocument>,
) -> Result<(), DocumentError> {
    let file = CatalogueFile {
        version: CATALOGUE_VERSION,
        documents: documents.values().map(document_to_file).collect(),
    };
    let bytes = serde_json::to_vec(&file).map_err(|_| DocumentError::Persist)?;
    if bytes.len() > MAXIMUM_CATALOGUE_BYTES {
        return Err(DocumentError::Full);
    }
    if let Some(path) = path {
        crate::storage::write_private(path, &bytes).map_err(|_| DocumentError::Persist)?;
    }
    Ok(())
}

fn document_to_file(document: &PlanDocument) -> DocumentFile {
    DocumentFile {
        id: document.id.as_hex(),
        kind: document.kind,
        title: document.title.clone(),
        associated_conversation: document.associated_conversation.map(|id| id.as_hex()),
        revisions: document.revisions.iter().map(revision_to_file).collect(),
        created_at_ms: document.created_at_ms,
        updated_at_ms: document.updated_at_ms,
    }
}

fn revision_to_file(revision: &PlanRevision) -> RevisionFile {
    RevisionFile {
        revision: revision.revision,
        content_hash: revision.content_hash.as_str(),
        object_hash: revision.object_hash.as_str(),
        artefact_hash: revision.artefact_hash.as_str(),
        content_bytes: revision.content_bytes,
        source: source_to_file(&revision.source),
        created_at_ms: revision.created_at_ms,
    }
}

fn source_to_file(source: &PlanSource) -> SourceFile {
    match source {
        PlanSource::ConversationMessage {
            conversation_id,
            message_index,
            source_hash,
        } => SourceFile::ConversationMessage {
            conversation_id: conversation_id.as_hex(),
            message_index: *message_index,
            source_hash: source_hash.as_str(),
        },
        PlanSource::SubmittedText {
            conversation_id,
            source_hash,
        } => SourceFile::SubmittedText {
            conversation_id: conversation_id.as_hex(),
            source_hash: source_hash.as_str(),
        },
        PlanSource::DirectoryFile {
            conversation_id,
            directory_id,
            path,
            source_hash,
        } => SourceFile::DirectoryFile {
            conversation_id: conversation_id.as_hex(),
            directory_id: directory_id.as_hex(),
            path: path.clone(),
            source_hash: source_hash.as_str(),
        },
        PlanSource::Correction { previous } => SourceFile::Correction {
            previous: RevisionReferenceFile {
                document_id: previous.document_id.as_hex(),
                revision: previous.revision,
                content_hash: previous.content_hash.as_str(),
                object_hash: previous.object_hash.as_str(),
                artefact_hash: previous.artefact_hash.as_str(),
            },
        },
    }
}

fn total_bytes(documents: &BTreeMap<DocumentId, PlanDocument>) -> u64 {
    documents
        .values()
        .flat_map(|document| document.revisions.iter())
        .map(|revision| revision.content_bytes)
        .sum()
}

fn unused_identifier(
    documents: &BTreeMap<DocumentId, PlanDocument>,
) -> Result<DocumentId, DocumentError> {
    for _ in 0..16 {
        let id = DocumentId::generate().map_err(|_| DocumentError::Random)?;
        if !documents.contains_key(&id) {
            return Ok(id);
        }
    }
    Err(DocumentError::Random)
}

fn normalise_title(raw: &str) -> Result<String, DocumentError> {
    let title = raw.trim();
    if title.is_empty()
        || title.len() > MAXIMUM_DOCUMENT_TITLE_BYTES
        || title.chars().any(char::is_control)
    {
        return Err(DocumentError::Title);
    }
    Ok(title.to_owned())
}

fn map_content_error(error: crate::workflows::artefacts::ArtefactStoreError) -> DocumentError {
    match error {
        crate::workflows::artefacts::ArtefactStoreError::Persist => DocumentError::Persist,
        crate::workflows::artefacts::ArtefactStoreError::Integrity
        | crate::workflows::artefacts::ArtefactStoreError::Missing => DocumentError::Corrupt,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;
