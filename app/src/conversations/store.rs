use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::agents::{AccessMode, AgentId, AgentRecord};
use crate::projects::ProjectId;
use crate::workflows::artefacts::{ArtefactHash, ObjectHash};

use super::access::ConversationGrant;
use super::documents::{DocumentId, PlanRevisionReference};
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
pub(crate) const MAXIMUM_PROJECT_ASSOCIATIONS: usize = 8;
const MAXIMUM_LINKED_REVIEWS: usize = 32;
const MAXIMUM_REVIEW_BRIEF_BYTES: usize = MAXIMUM_MESSAGE_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlanReviewLink {
    pub(crate) conversation_id: ConversationId,
    pub(crate) plan: PlanRevisionReference,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlanReviewContext {
    pub(crate) source: PlanReviewLink,
    pub(crate) task_brief: String,
}

pub(crate) struct PlanReviewCreation {
    pub(crate) source_id: ConversationId,
    pub(crate) source_revision: u32,
    pub(crate) title: String,
    pub(crate) model: ConversationModelConfiguration,
    pub(crate) plan: PlanRevisionReference,
    pub(crate) task_brief: String,
    pub(crate) read_only_projects: Vec<(ProjectId, u32)>,
    pub(crate) source_target: Option<ProjectId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationRecord {
    pub(crate) id: ConversationId,
    pub(crate) revision: u32,
    pub(crate) title: String,
    pub(crate) projects: Vec<ProjectId>,
    pub(crate) grants: Vec<ConversationGrant>,
    pub(crate) execution_target: Option<ProjectId>,
    pub(crate) network: crate::agents::NetworkAccess,
    pub(crate) model: Option<ConversationModelConfiguration>,
    pub(crate) source_review: Option<PlanReviewLink>,
    pub(crate) plan_reviews: Vec<PlanReviewLink>,
    pub(crate) review_context: Option<PlanReviewContext>,
    pub(crate) messages: Vec<ConversationMessage>,
    pub(crate) active_job: Option<JobId>,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationModelConfiguration {
    pub(crate) selection: ModelSelection,
    pub(crate) instructions: String,
    pub(crate) preset: Option<AppliedPreset>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppliedPreset {
    pub(crate) id: AgentId,
    pub(crate) revision: u32,
    pub(crate) name: String,
}

impl ConversationModelConfiguration {
    pub(crate) fn direct(selection: ModelSelection) -> Self {
        Self {
            selection,
            instructions: String::new(),
            preset: None,
        }
    }

    pub(crate) fn from_preset(record: &AgentRecord, selection: ModelSelection) -> Self {
        Self {
            selection,
            instructions: record.instructions.clone(),
            preset: Some(AppliedPreset {
                id: record.id,
                revision: record.revision,
                name: record.name.clone(),
            }),
        }
    }
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
    Projects,
    DuplicateProject,
    Access,
    Target,
    WriteTarget,
    Network,
    Review,
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
            Self::Projects => "This conversation can reference at most eight projects.",
            Self::DuplicateProject => "That project is already a context reference.",
            Self::Access => {
                "Grant project access to an attached project before inspection or changes."
            }
            Self::Target => "Choose a granted project as the execution target.",
            Self::WriteTarget => {
                "Only one project can have writable access in a conversation. Revoke the other writable grant first."
            }
            Self::Network => "Choose valid network access for this conversation.",
            Self::Review => "That plan review hand-off is no longer available.",
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
    projects: Vec<String>,
    grants: Vec<ConversationGrantFile>,
    #[serde(deserialize_with = "crate::storage::required_option")]
    execution_target: Option<String>,
    network: String,
    network_domains: Vec<String>,
    #[serde(deserialize_with = "crate::storage::required_option")]
    model: Option<ConversationModelFile>,
    #[serde(deserialize_with = "crate::storage::required_option")]
    source_review: Option<ReviewLinkFile>,
    plan_reviews: Vec<ReviewLinkFile>,
    #[serde(deserialize_with = "crate::storage::required_option")]
    review_context: Option<ReviewContextFile>,
    messages: Vec<MessageFile>,
    #[serde(deserialize_with = "crate::storage::required_option")]
    active_job: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ConversationGrantFile {
    project: String,
    project_revision: u32,
    authority_revision: u32,
    access: AccessMode,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ConversationModelFile {
    selection: ModelSelection,
    instructions: String,
    #[serde(deserialize_with = "crate::storage::required_option")]
    preset: Option<AppliedPresetFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct AppliedPresetFile {
    id: String,
    revision: u32,
    name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ReviewLinkFile {
    conversation: String,
    document: String,
    document_revision: u32,
    content_hash: String,
    object_hash: String,
    artefact_hash: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ReviewContextFile {
    source: ReviewLinkFile,
    task_brief: String,
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
            projects: Vec::new(),
            grants: Vec::new(),
            execution_target: None,
            network: crate::agents::NetworkAccess::None,
            model: None,
            source_review: None,
            plan_reviews: Vec::new(),
            review_context: None,
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

    pub(crate) fn create_plan_review(
        &self,
        creation: PlanReviewCreation,
    ) -> Result<ConversationRecord, ConversationError> {
        let PlanReviewCreation {
            source_id,
            source_revision,
            title,
            model,
            plan,
            task_brief,
            read_only_projects,
            source_target,
        } = creation;
        let title = normalise_title(&title)?;
        let task_brief = normalise_message(&task_brief)?;
        if task_brief.len() > MAXIMUM_REVIEW_BRIEF_BYTES
            || read_only_projects.len() > MAXIMUM_PROJECT_ASSOCIATIONS
        {
            return Err(ConversationError::Review);
        }
        let mut conversations = self.lock();
        let source = conversations
            .get(&source_id)
            .cloned()
            .ok_or(ConversationError::Missing)?;
        if source.revision != source_revision
            || source.active_job.is_some()
            || source.plan_reviews.len() >= MAXIMUM_LINKED_REVIEWS
        {
            return Err(if source.revision != source_revision {
                ConversationError::Conflict
            } else if source.active_job.is_some() {
                ConversationError::Active
            } else {
                ConversationError::Review
            });
        }
        if conversations.len() >= MAXIMUM_CONVERSATIONS {
            return Err(ConversationError::Full);
        }
        let mut projects = Vec::with_capacity(read_only_projects.len());
        let mut grants = Vec::with_capacity(read_only_projects.len());
        for (project_id, project_revision) in read_only_projects {
            if projects.contains(&project_id) || !source.projects.contains(&project_id) {
                return Err(ConversationError::Review);
            }
            let Some(source_grant) = source
                .grants
                .iter()
                .find(|grant| grant.project_id == project_id)
            else {
                return Err(ConversationError::Review);
            };
            if source_grant.project_revision != project_revision {
                return Err(ConversationError::Conflict);
            }
            projects.push(project_id);
            grants.push(ConversationGrant {
                project_id,
                project_revision,
                authority_revision: 1,
                access: AccessMode::ReadOnly,
            });
        }
        let id = unused_identifier(&conversations)?;
        let target = source_target
            .filter(|target| projects.contains(target))
            .or_else(|| projects.first().copied());
        let now = now_ms();
        let source_link = PlanReviewLink {
            conversation_id: source_id,
            plan: plan.clone(),
        };
        let review_link = PlanReviewLink {
            conversation_id: id,
            plan,
        };
        let review = ConversationRecord {
            id,
            revision: 1,
            title,
            projects,
            grants,
            execution_target: target,
            network: crate::agents::NetworkAccess::None,
            model: Some(model),
            source_review: Some(source_link.clone()),
            plan_reviews: Vec::new(),
            review_context: Some(PlanReviewContext {
                source: source_link,
                task_brief,
            }),
            messages: Vec::new(),
            active_job: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let mut updated_source = source.clone();
        updated_source.revision = source
            .revision
            .checked_add(1)
            .ok_or(ConversationError::Revision)?;
        updated_source.updated_at_ms = now.max(source.updated_at_ms);
        updated_source.plan_reviews.push(review_link);
        let previous = conversations.clone();
        conversations.insert(source_id, updated_source);
        conversations.insert(id, review.clone());
        if let Err(error) = persist(self.path.as_deref(), &conversations) {
            *conversations = previous;
            return Err(error);
        }
        Ok(review)
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

    pub(crate) fn attach_project(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        project: ProjectId,
    ) -> Result<ConversationRecord, ConversationError> {
        self.replace(id, expected_revision, |current| {
            if current.active_job.is_some() {
                return Err(ConversationError::Active);
            }
            if current.projects.contains(&project) {
                return Err(ConversationError::DuplicateProject);
            }
            if current.projects.len() >= MAXIMUM_PROJECT_ASSOCIATIONS {
                return Err(ConversationError::Projects);
            }
            current.projects.push(project);
            Ok(())
        })
    }

    pub(crate) fn detach_project(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        project: ProjectId,
    ) -> Result<ConversationRecord, ConversationError> {
        self.replace(id, expected_revision, |current| {
            if current.active_job.is_some() {
                return Err(ConversationError::Active);
            }
            let Some(index) = current.projects.iter().position(|item| *item == project) else {
                return Err(ConversationError::Missing);
            };
            current.projects.remove(index);
            current.grants.retain(|grant| grant.project_id != project);
            if current.execution_target == Some(project) {
                current.execution_target = None;
            }
            for grant in &mut current.grants {
                grant.authority_revision = current.revision;
            }
            Ok(())
        })
    }

    pub(crate) fn grant_read_only(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        project: ProjectId,
        project_revision: u32,
    ) -> Result<ConversationRecord, ConversationError> {
        self.grant_access(
            id,
            expected_revision,
            project,
            project_revision,
            AccessMode::ReadOnly,
        )
    }

    pub(crate) fn grant_writable(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        project: ProjectId,
        project_revision: u32,
    ) -> Result<ConversationRecord, ConversationError> {
        self.grant_access(
            id,
            expected_revision,
            project,
            project_revision,
            AccessMode::ReadWrite,
        )
    }

    pub(crate) fn grant_access(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        project: ProjectId,
        project_revision: u32,
        access: AccessMode,
    ) -> Result<ConversationRecord, ConversationError> {
        self.replace(id, expected_revision, |current| {
            if current.active_job.is_some() {
                return Err(ConversationError::Active);
            }
            if !current.projects.contains(&project) {
                return Err(ConversationError::Access);
            }
            if access.is_writable()
                && current
                    .grants
                    .iter()
                    .any(|grant| grant.project_id != project && grant.access.is_writable())
            {
                return Err(ConversationError::WriteTarget);
            }
            if let Some(grant) = current
                .grants
                .iter_mut()
                .find(|grant| grant.project_id == project)
            {
                grant.project_revision = project_revision;
                grant.authority_revision = current.revision;
                grant.access = access;
            } else {
                current.grants.push(ConversationGrant {
                    project_id: project,
                    project_revision,
                    authority_revision: current.revision,
                    access,
                });
            }
            if access.is_writable() || current.execution_target.is_none() {
                current.execution_target = Some(project);
            }
            for grant in &mut current.grants {
                grant.authority_revision = current.revision;
            }
            Ok(())
        })
    }

    pub(crate) fn select_execution_target(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        project: ProjectId,
    ) -> Result<ConversationRecord, ConversationError> {
        self.replace(id, expected_revision, |current| {
            if current.active_job.is_some() {
                return Err(ConversationError::Active);
            }
            if !current
                .grants
                .iter()
                .any(|grant| grant.project_id == project)
            {
                return Err(ConversationError::Target);
            }
            current.execution_target = Some(project);
            for grant in &mut current.grants {
                grant.authority_revision = current.revision;
            }
            Ok(())
        })
    }

    pub(crate) fn set_network(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        network: crate::agents::NetworkAccess,
    ) -> Result<ConversationRecord, ConversationError> {
        let network = network.validate().map_err(|_| ConversationError::Network)?;
        self.replace(id, expected_revision, |current| {
            if current.active_job.is_some() {
                return Err(ConversationError::Active);
            }
            current.network = network;
            for grant in &mut current.grants {
                grant.authority_revision = current.revision;
            }
            Ok(())
        })
    }

    pub(crate) fn select_model(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        selection: ModelSelection,
    ) -> Result<ConversationRecord, ConversationError> {
        self.select_model_configuration(
            id,
            expected_revision,
            ConversationModelConfiguration::direct(selection),
        )
    }

    pub(crate) fn apply_preset(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        preset: &AgentRecord,
        selection: ModelSelection,
    ) -> Result<ConversationRecord, ConversationError> {
        self.select_model_configuration(
            id,
            expected_revision,
            ConversationModelConfiguration::from_preset(preset, selection),
        )
    }

    pub(crate) fn select_model_configuration(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        model: ConversationModelConfiguration,
    ) -> Result<ConversationRecord, ConversationError> {
        self.replace(id, expected_revision, |current| {
            if current.active_job.is_some() {
                return Err(ConversationError::Active);
            }
            current.model = Some(model);
            Ok(())
        })
    }

    pub(crate) fn begin_message_with_model(
        &self,
        id: &ConversationId,
        expected_revision: u32,
        model: ConversationModelConfiguration,
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
            current.model = Some(model);
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
        || file.projects.len() > MAXIMUM_PROJECT_ASSOCIATIONS
        || file.grants.len() > MAXIMUM_PROJECT_ASSOCIATIONS
    {
        return Err(ConversationError::Corrupt);
    }
    let title = normalise_title(&file.title).map_err(|_| ConversationError::Corrupt)?;
    if title != file.title {
        return Err(ConversationError::Corrupt);
    }
    let network = parse_stored_network(&file.network, &file.network_domains)?;
    let model = file.model.map(model_from_file).transpose()?;
    let source_review = file.source_review.map(review_link_from_file).transpose()?;
    let plan_reviews = file
        .plan_reviews
        .into_iter()
        .map(review_link_from_file)
        .collect::<Result<Vec<_>, _>>()?;
    if plan_reviews.len() > MAXIMUM_LINKED_REVIEWS
        || plan_reviews.iter().enumerate().any(|(index, link)| {
            plan_reviews[..index]
                .iter()
                .any(|previous| previous == link)
        })
    {
        return Err(ConversationError::Corrupt);
    }
    let review_context = file
        .review_context
        .map(review_context_from_file)
        .transpose()?;
    if review_context
        .as_ref()
        .is_some_and(|context| source_review.as_ref() != Some(&context.source))
    {
        return Err(ConversationError::Corrupt);
    }
    let mut projects = Vec::with_capacity(file.projects.len());
    for raw in file.projects {
        let project = ProjectId::parse(&raw).ok_or(ConversationError::Corrupt)?;
        if projects.contains(&project) {
            return Err(ConversationError::Corrupt);
        }
        projects.push(project);
    }
    let mut grants = Vec::with_capacity(file.grants.len());
    for grant in file.grants {
        let project_id = ProjectId::parse(&grant.project).ok_or(ConversationError::Corrupt)?;
        if grant.project_revision == 0
            || grant.authority_revision == 0
            || !projects.contains(&project_id)
            || grants
                .iter()
                .any(|item: &ConversationGrant| item.project_id == project_id)
        {
            return Err(ConversationError::Corrupt);
        }
        grants.push(ConversationGrant {
            project_id,
            project_revision: grant.project_revision,
            authority_revision: grant.authority_revision,
            access: grant.access,
        });
    }
    if file
        .execution_target
        .as_ref()
        .is_some_and(|target| ProjectId::parse(target).is_none())
    {
        return Err(ConversationError::Corrupt);
    }
    let execution_target = file.execution_target.as_deref().and_then(ProjectId::parse);
    if execution_target.is_some_and(|target| !grants.iter().any(|grant| grant.project_id == target))
    {
        return Err(ConversationError::Corrupt);
    }
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
    if grants
        .iter()
        .filter(|grant| grant.access.is_writable())
        .count()
        > 1
    {
        return Err(ConversationError::Corrupt);
    }
    Ok(ConversationRecord {
        id,
        revision: file.revision,
        title,
        projects,
        grants,
        execution_target,
        network,
        model,
        source_review,
        plan_reviews,
        review_context,
        messages,
        active_job,
        created_at_ms: file.created_at_ms,
        updated_at_ms: file.updated_at_ms,
    })
}

fn model_from_file(
    file: ConversationModelFile,
) -> Result<ConversationModelConfiguration, ConversationError> {
    let selection = ModelSelection::new(
        file.selection.provider,
        file.selection.model.clone(),
        file.selection.thinking.clone(),
    )
    .filter(|selection| selection == &file.selection)
    .ok_or(ConversationError::Corrupt)?;
    if file.instructions.len() > crate::agents::MAXIMUM_INSTRUCTION_BYTES
        || file
            .instructions
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(ConversationError::Corrupt);
    }
    let preset = match file.preset {
        Some(preset)
            if preset.revision > 0
                && !preset.name.trim().is_empty()
                && preset.name.len() <= crate::agents::MAXIMUM_NAME_BYTES
                && !preset.name.chars().any(char::is_control) =>
        {
            Some(AppliedPreset {
                id: AgentId::parse(&preset.id).ok_or(ConversationError::Corrupt)?,
                revision: preset.revision,
                name: preset.name,
            })
        }
        None => None,
        Some(_) => return Err(ConversationError::Corrupt),
    };
    Ok(ConversationModelConfiguration {
        selection,
        instructions: file.instructions,
        preset,
    })
}

fn model_to_file(model: &ConversationModelConfiguration) -> ConversationModelFile {
    ConversationModelFile {
        selection: model.selection.clone(),
        instructions: model.instructions.clone(),
        preset: model.preset.as_ref().map(|preset| AppliedPresetFile {
            id: preset.id.as_hex(),
            revision: preset.revision,
            name: preset.name.clone(),
        }),
    }
}

fn review_link_from_file(file: ReviewLinkFile) -> Result<PlanReviewLink, ConversationError> {
    let conversation_id =
        ConversationId::parse(&file.conversation).ok_or(ConversationError::Corrupt)?;
    let document_id = DocumentId::parse(&file.document).ok_or(ConversationError::Corrupt)?;
    if file.document_revision == 0 {
        return Err(ConversationError::Corrupt);
    }
    Ok(PlanReviewLink {
        conversation_id,
        plan: PlanRevisionReference {
            document_id,
            revision: file.document_revision,
            content_hash: ObjectHash::parse(&file.content_hash)
                .ok_or(ConversationError::Corrupt)?,
            object_hash: ObjectHash::parse(&file.object_hash).ok_or(ConversationError::Corrupt)?,
            artefact_hash: ArtefactHash::parse(&file.artefact_hash)
                .ok_or(ConversationError::Corrupt)?,
        },
    })
}

fn review_link_to_file(link: &PlanReviewLink) -> ReviewLinkFile {
    ReviewLinkFile {
        conversation: link.conversation_id.as_hex(),
        document: link.plan.document_id.as_hex(),
        document_revision: link.plan.revision,
        content_hash: link.plan.content_hash.as_str(),
        object_hash: link.plan.object_hash.as_str(),
        artefact_hash: link.plan.artefact_hash.as_str(),
    }
}

fn review_context_from_file(
    file: ReviewContextFile,
) -> Result<PlanReviewContext, ConversationError> {
    let task_brief = normalise_message(&file.task_brief)?;
    if task_brief.len() > MAXIMUM_REVIEW_BRIEF_BYTES {
        return Err(ConversationError::Corrupt);
    }
    Ok(PlanReviewContext {
        source: review_link_from_file(file.source)?,
        task_brief,
    })
}

fn review_context_to_file(context: &PlanReviewContext) -> ReviewContextFile {
    ReviewContextFile {
        source: review_link_to_file(&context.source),
        task_brief: context.task_brief.clone(),
    }
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
        projects: record.projects.iter().map(ProjectId::as_hex).collect(),
        grants: record
            .grants
            .iter()
            .map(|grant| ConversationGrantFile {
                project: grant.project_id.as_hex(),
                project_revision: grant.project_revision,
                authority_revision: grant.authority_revision,
                access: grant.access,
            })
            .collect(),
        execution_target: record.execution_target.map(|project| project.as_hex()),
        network: record.network.as_str().to_owned(),
        network_domains: record.network.domains().to_vec(),
        model: record.model.as_ref().map(model_to_file),
        source_review: record.source_review.as_ref().map(review_link_to_file),
        plan_reviews: record
            .plan_reviews
            .iter()
            .map(review_link_to_file)
            .collect(),
        review_context: record.review_context.as_ref().map(review_context_to_file),
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

fn parse_stored_network(
    mode: &str,
    domains: &[String],
) -> Result<crate::agents::NetworkAccess, ConversationError> {
    match mode {
        "none" if domains.is_empty() => Ok(crate::agents::NetworkAccess::None),
        "restricted" => crate::agents::NetworkAccess::parse_form(mode, &domains.join("\n"))
            .map_err(|_| ConversationError::Corrupt),
        "public" if domains.is_empty() => Ok(crate::agents::NetworkAccess::Public),
        _ => Err(ConversationError::Corrupt),
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
