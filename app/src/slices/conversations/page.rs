mod model_picker;

use askama::Template;
use model_picker::ModelPicker;

#[cfg(test)]
mod tests;

use crate::{
    agents::{AgentRecord, NetworkAccess, ToolId},
    conversations::{
        ConversationMessage, ConversationModelConfiguration, ConversationRecord,
        MAXIMUM_PROJECT_ASSOCIATIONS, MessageRole, MessageStatus, PlanDocument, PlanSource,
    },
    environments::{
        EnvironmentCatalogue, EnvironmentId, EnvironmentSnapshotRepository, PreparationState,
        SnapshotAvailability,
    },
    models::models_dev::ModelsDevCatalogue,
    projects::{ProjectId, ProjectRecord},
    providers::ModelSelection,
    sessions::{JobSnapshot, JobStatus},
    vault::ProviderVault,
    workflows::WorkflowRun,
};

pub(super) const CATALOGUE_TITLE: &str = "Conversations | Power Plant";

#[derive(Template)]
#[template(source = "{{ message }}", ext = "html")]
pub(super) struct ModelSelectionStatus<'a> {
    pub(super) message: &'a str,
}

pub(super) struct ConversationListItem {
    pub(super) id: String,
    pub(super) title: String,
}

pub(super) struct CatalogueProjectOption {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) selected: bool,
}

pub(super) struct DirectoryView {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) alias: String,
    pub(super) host_path: String,
    pub(super) guest_path: String,
    pub(super) form_value: String,
    pub(super) available: bool,
    pub(super) sensitive: bool,
    pub(super) pending_approval: bool,
    pub(super) transient: bool,
}

pub(super) struct ProjectContextView {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) alias: String,
    pub(super) status: &'static str,
    pub(super) access_granted: bool,
    pub(super) writable: bool,
    pub(super) execution_target: bool,
    pub(super) secondary_context: bool,
}

pub(super) struct CandidateChangeView {
    pub(super) path: String,
    pub(super) status: &'static str,
}

pub(super) struct PendingCodeGateView {
    pub(super) run_id: String,
    pub(super) gate_id: String,
    pub(super) revision: String,
    pub(super) candidate: String,
    pub(super) diff_base: String,
    pub(super) diff_href: String,
    pub(super) review_href: String,
    pub(super) changes: Vec<CandidateChangeView>,
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "conversation_catalogue"
)]
pub(super) struct CatalogueView {
    pub(super) conversations: Vec<ConversationListItem>,
    pub(super) projects: Vec<CatalogueProjectOption>,
    pub(super) filter: String,
    pub(super) error: &'static str,
}

impl CatalogueView {
    pub(super) fn from_records(
        records: &[ConversationRecord],
        project_records: &[ProjectRecord],
        filter: Option<ProjectId>,
        error: &'static str,
    ) -> Self {
        let mut conversations: Vec<_> = records
            .iter()
            .filter(|record| filter.is_none_or(|project| record.projects.contains(&project)))
            .map(|record| ConversationListItem {
                id: record.id.as_hex(),
                title: record.title.clone(),
            })
            .collect();
        conversations.sort_by(|left, right| left.title.cmp(&right.title));
        let mut projects: Vec<_> = project_records
            .iter()
            .map(|project| CatalogueProjectOption {
                id: project.id.as_hex(),
                name: project.name.clone(),
                selected: filter.is_some_and(|selected| selected == project.id),
            })
            .collect();
        projects.sort_by(|left, right| left.name.cmp(&right.name));
        Self {
            conversations,
            projects,
            filter: filter.map_or_else(String::new, |project| project.as_hex()),
            error,
        }
    }
}

pub(super) struct ReviewProjectOption {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) access: &'static str,
    pub(super) selected: bool,
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "plan_review_page"
)]
pub(super) struct PlanReviewView {
    pub(super) document_title: String,
    pub(super) source_title: String,
    pub(super) source_id: String,
    pub(super) source_revision: String,
    pub(super) document_id: String,
    pub(super) document_revision: u32,
    pub(super) content_hash: String,
    pub(super) content_html: String,
    pub(super) brief: String,
    pub(super) reviewer_summary: String,
    pub(super) providers: Vec<ProviderOption>,
    pub(super) presets: Vec<PresetOption>,
    pub(super) read_only_projects: Vec<ReviewProjectOption>,
    pub(super) error: &'static str,
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "plan_review_detail"
)]
pub(super) struct PlanReviewContents<'a> {
    pub(super) source_title: &'a str,
    pub(super) source_id: &'a str,
    pub(super) source_revision: &'a str,
    pub(super) document_id: &'a str,
    pub(super) document_revision: u32,
    pub(super) content_hash: &'a str,
    pub(super) content_html: &'a str,
    pub(super) brief: &'a str,
    pub(super) reviewer_summary: &'a str,
    pub(super) providers: &'a [ProviderOption],
    pub(super) presets: &'a [PresetOption],
    pub(super) read_only_projects: &'a [ReviewProjectOption],
    pub(super) error: &'static str,
}

impl PlanReviewView {
    pub(super) fn contents(&self) -> PlanReviewContents<'_> {
        PlanReviewContents {
            source_title: &self.source_title,
            source_id: &self.source_id,
            source_revision: &self.source_revision,
            document_id: &self.document_id,
            document_revision: self.document_revision,
            content_hash: &self.content_hash,
            content_html: &self.content_html,
            brief: &self.brief,
            reviewer_summary: &self.reviewer_summary,
            providers: &self.providers,
            presets: &self.presets,
            read_only_projects: &self.read_only_projects,
            error: self.error,
        }
    }
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "candidate_review_page"
)]
pub(super) struct CandidateReviewView {
    pub(super) run_id: String,
    pub(super) source_title: String,
    pub(super) candidate_id: String,
    pub(super) diff_base_id: String,
    pub(super) candidate_hash: String,
    pub(super) diff_base_hash: String,
    pub(super) preview: String,
    pub(super) instructions_summary: String,
    pub(super) brief: String,
    pub(super) reviewer_summary: String,
    pub(super) providers: Vec<ProviderOption>,
    pub(super) presets: Vec<PresetOption>,
    pub(super) error: &'static str,
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "candidate_review_detail"
)]
pub(super) struct CandidateReviewContents<'a> {
    pub(super) run_id: &'a str,
    pub(super) source_title: &'a str,
    pub(super) candidate_id: &'a str,
    pub(super) diff_base_id: &'a str,
    pub(super) candidate_hash: &'a str,
    pub(super) diff_base_hash: &'a str,
    pub(super) preview: &'a str,
    pub(super) instructions_summary: &'a str,
    pub(super) brief: &'a str,
    pub(super) reviewer_summary: &'a str,
    pub(super) providers: &'a [ProviderOption],
    pub(super) presets: &'a [PresetOption],
    pub(super) error: &'static str,
}

impl CandidateReviewView {
    pub(super) fn contents(&self) -> CandidateReviewContents<'_> {
        CandidateReviewContents {
            run_id: &self.run_id,
            source_title: &self.source_title,
            candidate_id: &self.candidate_id,
            diff_base_id: &self.diff_base_id,
            candidate_hash: &self.candidate_hash,
            diff_base_hash: &self.diff_base_hash,
            preview: &self.preview,
            instructions_summary: &self.instructions_summary,
            brief: &self.brief,
            reviewer_summary: &self.reviewer_summary,
            providers: &self.providers,
            presets: &self.presets,
            error: self.error,
        }
    }
}

pub(super) struct MessageView {
    pub(super) index: usize,
    pub(super) user: bool,
    pub(super) html: String,
    pub(super) status: &'static str,
    pub(super) error: String,
    pub(super) streaming: bool,
    pub(super) saveable_plan: bool,
    pub(super) task_title: String,
    pub(super) task_action: String,
    pub(super) plan_title: String,
    pub(super) plan_action: String,
    pub(super) conversation_revision: String,
}

pub(super) struct PlanDocumentView {
    pub(super) title: String,
    pub(super) kind: String,
    pub(super) task_list: bool,
    pub(super) prepare_href: String,
    pub(super) revision: u32,
    pub(super) provenance: String,
    pub(super) content_hash: String,
    pub(super) open_href: String,
    pub(super) export_href: String,
    pub(super) review_href: String,
    pub(super) remove_href: String,
}

pub(super) struct ConversationLinkView {
    pub(super) title: String,
    pub(super) plan_title: String,
    pub(super) href: String,
    pub(super) plan_href: String,
    pub(super) plan_revision: u32,
    pub(super) content_hash: String,
}

pub(super) struct CandidateReviewLinkView {
    pub(super) title: String,
    pub(super) href: String,
    pub(super) run_href: String,
    pub(super) candidate_hash: String,
    pub(super) diff_base_hash: String,
}

pub(super) struct WorkflowProgressView {
    pub(super) run_href: String,
    pub(super) name: String,
    pub(super) state: &'static str,
    pub(super) current_step: String,
    pub(super) result: &'static str,
    pub(super) task_progress: String,
    pub(super) loop_id: String,
    pub(super) command_token: String,
    pub(super) can_pause: bool,
    pub(super) can_continue: bool,
    pub(super) can_retry: bool,
    pub(super) can_stop: bool,
    pub(super) pause_requested: bool,
    pub(super) awaiting_gate: bool,
}

pub(super) struct ModelSources<'a> {
    pub(super) vault: &'a ProviderVault,
    pub(super) preferences: &'a crate::preferences::Preferences,
    pub(super) models: &'a ModelsDevCatalogue,
    pub(super) environments: &'a EnvironmentCatalogue,
    pub(super) environment_snapshots: &'a EnvironmentSnapshotRepository,
    pub(super) projects: &'a [ProjectRecord],
    pub(super) documents: &'a [PlanDocument],
}

pub(super) struct PresetOption {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) selected: bool,
}

pub(super) struct ProviderOption {
    pub(super) value: &'static str,
    pub(super) label: &'static str,
    pub(super) model: String,
    pub(super) thinking: String,
    pub(super) selected: bool,
}

#[derive(Clone)]
pub(super) struct NetworkOption {
    pub(super) value: &'static str,
    pub(super) label: &'static str,
    pub(super) selected: bool,
}

pub(super) struct EnvironmentOption {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) readiness: &'static str,
    pub(super) availability: &'static str,
    pub(super) selected: bool,
}

pub(super) struct ToolOption {
    pub(super) field_name: &'static str,
    pub(super) value: &'static str,
    pub(super) label: &'static str,
    pub(super) detail: &'static str,
    pub(super) selected: bool,
}

pub(super) struct SubmittedSettingsFields<'a> {
    pub(super) provider: &'a str,
    pub(super) model: &'a str,
    pub(super) thinking: &'a str,
    pub(super) instructions: String,
    pub(super) tools: Vec<String>,
    pub(super) environment: &'a str,
    pub(super) network: &'a str,
    pub(super) network_domains: &'a str,
}

pub(super) enum ConversationPageState {
    New { project: String, message: String },
    Saved(Box<SavedConversationState>),
}

#[derive(Template)]
#[template(path = "conversations/templates/detail.html", blocks = ["conversation_detail"])]
pub(super) struct ConversationDetailView {
    pub(super) heading: String,
    pub(super) document_title: String,
    pub(super) title: String,
    pub(super) state: ConversationPageState,
    pub(super) error: &'static str,
    pub(super) messages: Vec<MessageView>,
    pub(super) omitted_messages: usize,
    model_picker: ModelPicker,
    pub(super) presets: Vec<PresetOption>,
    pub(super) attached_projects: Vec<ProjectContextView>,
    pub(super) attachable_projects: Vec<CatalogueProjectOption>,
    pub(super) directories: Vec<DirectoryView>,
    pub(super) data_root: String,
    pub(super) consent_path: String,
    pub(super) consent_request: String,
    pub(super) pending_directory: String,
    pub(super) consent_existing: bool,
    pub(super) draft_nonce: String,
    pub(super) consent_reference: String,
    pub(super) model_summary: String,
    pub(super) instructions: String,
    pub(super) tool_options: Vec<ToolOption>,
    pub(super) environment_options: Vec<EnvironmentOption>,
    pub(super) environment_summary: String,
    pub(super) network_options: Vec<NetworkOption>,
    pub(super) network_domains: String,
    pub(super) network_summary: String,
    pub(super) network_detail: String,
    pub(super) settings_open: bool,
    pub(super) directories_open: bool,
    pub(super) model_available: bool,
    pub(super) job_active: bool,
    pub(super) session_busy: bool,
}

pub(super) struct SavedConversationState {
    pub(super) id: String,
    pub(super) revision: String,
    pub(super) project_limit_reached: bool,
    pub(super) job_id: String,
    pub(super) cursor: u64,
    pub(super) pending_gate: Option<PendingCodeGateView>,
    pub(super) plans: Vec<PlanDocumentView>,
    pub(super) task_text: String,
    pub(super) task_title: String,
    pub(super) source_review: Option<ConversationLinkView>,
    pub(super) linked_reviews: Vec<ConversationLinkView>,
    pub(super) source_candidate_review: Option<CandidateReviewLinkView>,
    pub(super) linked_candidate_reviews: Vec<CandidateReviewLinkView>,
    pub(super) workflow_progress: Option<WorkflowProgressView>,
}
impl ConversationDetailView {
    pub(super) fn from_new(
        state: &crate::state::AppState,
        session: crate::sessions::SessionId,
        form: super::new::NewForm,
        error: &'static str,
    ) -> Self {
        let selected_tools = form.tool_values();
        let draft_directories = form.directories().unwrap_or_default();
        let mut directories = directory_views(&draft_directories);
        for (view, grant) in directories.iter_mut().zip(&draft_directories) {
            view.sensitive = crate::execution::authority::sensitive_directory(
                &grant.host_path,
                state.local_data.root(),
            );
            view.pending_approval = view.sensitive
                && !state.access_consent.authorised_draft(
                    &form.consent_reference,
                    session,
                    &form.consent_nonce(),
                    &draft_directories,
                    grant,
                );
        }
        let consent_path = form
            .pending_directory()
            .map(|grant| grant.host_path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let consent_existing = form.consent_existing == "true";
        if !consent_existing && let Some(grant) = form.pending_directory() {
            let mut view = directory_view(&grant);
            view.sensitive = true;
            view.pending_approval = true;
            view.transient = true;
            directories.push(view);
        }
        Self {
            heading: "New conversation".to_owned(),
            document_title: "New conversation | Power Plant".to_owned(),
            title: form.title,
            model_picker: ModelPicker::new(
                &state.vault,
                &state.preferences,
                &state.models_dev,
                &form.provider,
                &form.model,
                &form.thinking,
            ),
            presets: state
                .agents
                .list()
                .into_iter()
                .map(|agent| PresetOption {
                    id: agent.id.as_hex(),
                    name: agent.name,
                    description: String::new(),
                    selected: agent.id.as_hex() == form.preset,
                })
                .collect(),
            attachable_projects: state
                .projects
                .list()
                .into_iter()
                .map(|project| CatalogueProjectOption {
                    id: project.id.as_hex(),
                    name: project.name,
                    selected: project.id.as_hex() == form.project,
                })
                .collect(),
            state: ConversationPageState::New {
                project: form.project,
                message: form.message,
            },
            error,
            messages: Vec::new(),
            omitted_messages: 0,
            attached_projects: Vec::new(),
            directories,
            data_root: state.local_data.root().to_string_lossy().into_owned(),
            consent_path,
            consent_request: form.consent_request,
            pending_directory: form.pending_directory,
            consent_existing,
            draft_nonce: form.draft_nonce,
            consent_reference: form.consent_reference,
            model_summary: String::new(),
            instructions: form.instructions.clone(),
            tool_options: tool_options(&selected_tools),
            environment_options: environment_options(
                &state.environments,
                &state.environment_snapshots,
                EnvironmentId::parse(&form.environment),
            ),
            environment_summary: environment_summary(
                &state.environments,
                &state.environment_snapshots,
                EnvironmentId::parse(&form.environment),
            ),
            network_options: network_options(&form.network),
            network_domains: form.network_domains,
            network_summary: network_summary_from_form(&form.network),
            network_detail: String::new(),
            settings_open: false,
            directories_open: false,
            model_available: !form.model.is_empty(),
            job_active: false,
            session_busy: false,
        }
    }

    pub(super) fn saved(&self) -> Option<&SavedConversationState> {
        match &self.state {
            ConversationPageState::New { .. } => None,
            ConversationPageState::Saved(saved) => Some(saved),
        }
    }

    fn is_new(&self) -> bool {
        self.saved().is_none()
    }

    fn draft_project(&self) -> &str {
        match &self.state {
            ConversationPageState::New { project, .. } => project,
            ConversationPageState::Saved(_) => "",
        }
    }

    fn draft_message(&self) -> &str {
        match &self.state {
            ConversationPageState::New { message, .. } => message,
            ConversationPageState::Saved(_) => "",
        }
    }

    fn model_form(&self) -> &'static str {
        if self.is_new() {
            "conversation-composer"
        } else {
            "conversation-settings-form"
        }
    }

    fn composer_action(&self) -> String {
        self.saved().map_or_else(
            || "/conversations/new".to_owned(),
            |saved| format!("/conversations/{}/messages", saved.id),
        )
    }

    fn composer_disabled(&self) -> bool {
        !self.is_new() && (self.job_active || self.session_busy || !self.model_available)
    }

    fn transcript_empty(&self) -> bool {
        self.messages.is_empty()
            && self.saved().is_none_or(|saved| {
                saved.workflow_progress.is_none()
                    && saved.pending_gate.is_none()
                    && saved.source_review.is_none()
                    && saved.source_candidate_review.is_none()
            })
    }

    pub(super) fn with_task_text(mut self, title: String, text: String) -> Self {
        if let ConversationPageState::Saved(saved) = &mut self.state {
            saved.task_title = title;
            saved.task_text = text;
        }
        self
    }

    #[cfg(test)]
    pub(super) fn from_record(
        record: &ConversationRecord,
        sources: ModelSources<'_>,
        agents: &[AgentRecord],
        job: Option<&JobSnapshot>,
        session_busy: bool,
        title: &str,
        error: &'static str,
    ) -> Self {
        Self::from_record_with_gate(
            record,
            sources,
            agents,
            job,
            session_busy,
            title,
            error,
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_record_with_gate(
        record: &ConversationRecord,
        sources: ModelSources<'_>,
        agents: &[AgentRecord],
        job: Option<&JobSnapshot>,
        session_busy: bool,
        title: &str,
        error: &'static str,
        pending_gate: Option<PendingCodeGateView>,
        source_review: Option<ConversationLinkView>,
        linked_reviews: Vec<ConversationLinkView>,
        source_candidate_review: Option<CandidateReviewLinkView>,
        linked_candidate_reviews: Vec<CandidateReviewLinkView>,
    ) -> Self {
        let fallback = sources
            .preferences
            .desk_providers(sources.vault)
            .into_iter()
            .find(|provider| provider.selected)
            .and_then(|connection| {
                crate::workflows::alpine_git_id(sources.environments)
                    .ok()
                    .map(|environment| {
                        ConversationModelConfiguration::direct(
                            ModelSelection {
                                provider: connection.kind,
                                thinking: sources.models.effective_effort(
                                    connection.kind,
                                    &connection.model,
                                    connection.thinking.as_ref(),
                                ),
                                model: connection.model,
                            },
                            environment,
                        )
                    })
            });
        let configuration = record.model.as_ref().or(fallback.as_ref());
        let selection = configuration.map(|configuration| &configuration.settings.model);
        let selected_environment = configuration.map(|model| model.settings.environment);
        let network_options = network_options(record.network.as_str());
        let network_domains = record.network.domains().join("\n");
        let preset_network = configuration
            .and_then(|configuration| configuration.preset.as_ref())
            .and_then(|selected| agents.iter().find(|agent| agent.id == selected.id))
            .map(|agent| &agent.network);
        let effective_network =
            crate::conversations::intersect_network(&record.network, preset_network);
        let network_summary = network_summary_from_form(effective_network.as_str());
        let network_detail = format_network_summary(&record.network, &effective_network);
        let model_picker = ModelPicker::new(
            sources.vault,
            sources.preferences,
            sources.models,
            selection.map_or("", |selection| selection.provider.as_str()),
            selection.map_or("", |selection| selection.model.as_str()),
            selection
                .and_then(|selection| selection.thinking.as_ref())
                .map_or("", |effort| effort.as_str()),
        );
        let presets = agents
            .iter()
            .map(|agent| PresetOption {
                id: agent.id.as_hex(),
                name: agent.name.clone(),
                description: agent.selection.as_ref().map_or_else(
                    || "Keep the current model".to_owned(),
                    |selection| format!("{} · {}", selection.provider.label(), selection.model),
                ),
                selected: configuration
                    .and_then(|configuration| configuration.preset.as_ref())
                    .is_some_and(|preset| preset.id == agent.id),
            })
            .collect();
        let plans: Vec<_> = sources
            .documents
            .iter()
            .filter(|document| document.associated_conversation == Some(record.id))
            .map(plan_document_view)
            .collect();
        let attached_projects = record
            .projects
            .iter()
            .map(|id| {
                let access = record
                    .grants
                    .iter()
                    .find(|grant| grant.project_id == *id)
                    .map(|grant| grant.access);
                let access_granted = access.is_some();
                let writable = access == Some(crate::agents::AccessMode::ReadWrite);
                let execution_target = record.execution_target == Some(*id);
                let secondary_context = access_granted && !execution_target;
                let alias = if secondary_context {
                    crate::conversations::secondary_alias(*id)
                } else {
                    String::new()
                };
                match sources.projects.iter().find(|project| project.id == *id) {
                    Some(project) if project.host_path_is_available() => ProjectContextView {
                        id: id.as_hex(),
                        name: project.name.clone(),
                        alias: alias.clone(),
                        status: if execution_target && writable {
                            "Writable access granted. This is the execution target. Tools: List, Read, Run and Write in the candidate workspace. Network: None by default."
                        } else if secondary_context {
                            "Read-only context. Tools: List, Read and Run. Writes are blocked. Network: None by default."
                        } else if access_granted {
                            "Read-only access granted. This is the execution target. Tools: List, Read and Run. Network: None by default."
                        } else {
                            "Context reference only. File access is not granted."
                        },
                        access_granted,
                        writable,
                        execution_target,
                        secondary_context,
                    },
                    Some(project) => ProjectContextView {
                        id: id.as_hex(),
                        name: project.name.clone(),
                        alias: String::new(),
                        status: "Project unavailable. It remains a context reference without file access.",
                        access_granted: false,
                        writable: false,
                        execution_target: false,
                        secondary_context: false,
                    },
                    None => ProjectContextView {
                        id: id.as_hex(),
                        name: "Project record unavailable".to_owned(),
                        alias: String::new(),
                        status: "This context reference has no file access.",
                        access_granted: false,
                        writable: false,
                        execution_target: false,
                        secondary_context: false,
                    },
                }
            })
            .collect();
        let mut attachable_projects: Vec<_> = sources
            .projects
            .iter()
            .filter(|project| !record.projects.contains(&project.id))
            .map(|project| CatalogueProjectOption {
                id: project.id.as_hex(),
                name: project.name.clone(),
                selected: false,
            })
            .collect();
        attachable_projects.sort_by(|left, right| left.name.cmp(&right.name));
        let model_summary = configuration.map_or_else(
            || "No model selected".to_owned(),
            |configuration| {
                let selection = &configuration.settings.model;
                let effort = selection
                    .thinking
                    .as_ref()
                    .map(|effort| format!(" · Thinking: {}", effort.label()))
                    .unwrap_or_else(|| " · Thinking: Not available".to_owned());
                let source = configuration.preset.as_ref().map_or_else(
                    || "Direct model".to_owned(),
                    |preset| format!("Preset: {}", preset.name),
                );
                format!(
                    "{source} · {} · {}{effort}",
                    selection.provider.label(),
                    selection.model
                )
            },
        );
        let (job_id, cursor, job_active) = match job {
            Some(job) if job.status == JobStatus::Running => {
                (job.id.as_hex(), job.latest_seq, true)
            }
            _ => (String::new(), 0, false),
        };
        // Plan controls and the escaped model catalogue share the transcript envelope.
        let message_budget = (800_usize * 1024)
            .saturating_sub(plans.len() * 3072)
            .saturating_sub(ammonia::clean_text(&model_picker.catalogue).len());
        let messages = visible_messages(record, message_budget);
        let omitted_messages = record.messages.len() - messages.len();
        Self {
            heading: record.title.clone(),
            document_title: format!("{} | Power Plant", record.title),
            title: title.to_owned(),
            error,
            messages,
            omitted_messages,
            model_available: selection.is_some(),
            model_picker,
            presets,
            attached_projects,
            attachable_projects,
            directories: configuration
                .map(|configuration| directory_views(&configuration.settings.directories))
                .unwrap_or_default(),
            data_root: String::new(),
            consent_path: String::new(),
            consent_request: String::new(),
            pending_directory: String::new(),
            consent_existing: false,
            draft_nonce: String::new(),
            consent_reference: String::new(),
            model_summary,
            instructions: configuration
                .map(|configuration| configuration.settings.instructions.clone())
                .unwrap_or_default(),
            tool_options: tool_options(
                &configuration
                    .map(|configuration| {
                        configuration
                            .settings
                            .tools
                            .iter()
                            .map(|tool| tool.as_str().to_owned())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            ),
            environment_options: environment_options(
                sources.environments,
                sources.environment_snapshots,
                selected_environment,
            ),
            environment_summary: environment_summary(
                sources.environments,
                sources.environment_snapshots,
                selected_environment,
            ),
            network_options: network_options.clone(),
            network_domains: network_domains.clone(),
            network_summary,
            network_detail,
            settings_open: false,
            directories_open: false,
            job_active,
            session_busy,
            state: ConversationPageState::Saved(Box::new(SavedConversationState {
                id: record.id.as_hex(),
                revision: record.revision.to_string(),
                project_limit_reached: record.projects.len() >= MAXIMUM_PROJECT_ASSOCIATIONS,
                job_id,
                cursor,
                pending_gate,
                plans,
                task_text: String::new(),
                task_title: String::new(),
                source_review,
                linked_reviews,
                source_candidate_review,
                linked_candidate_reviews,
                workflow_progress: None,
            })),
        }
    }

    pub(super) fn with_access_status(
        mut self,
        state: &crate::state::AppState,
        session: crate::sessions::SessionId,
        record: &ConversationRecord,
    ) -> Self {
        self.data_root = state.local_data.root().to_string_lossy().into_owned();
        if let Some(configuration) = &record.model {
            for (view, grant) in self
                .directories
                .iter_mut()
                .zip(&configuration.settings.directories)
            {
                view.sensitive = crate::execution::authority::sensitive_directory(
                    &grant.host_path,
                    state.local_data.root(),
                );
                view.pending_approval = view.sensitive
                    && !state.access_consent.authorised_conversation(
                        session,
                        record.id,
                        &configuration.settings,
                        grant,
                    );
            }
        }
        self
    }

    pub(super) fn with_pending_directory(
        mut self,
        state: &crate::state::AppState,
        grant: crate::execution::DirectoryGrant,
        request: String,
        existing: bool,
    ) -> Self {
        if existing {
            if let Some(view) = self
                .directories
                .iter_mut()
                .find(|view| view.id == grant.id.as_hex())
            {
                view.sensitive = true;
                view.pending_approval = true;
            }
        } else {
            let mut view = directory_view(&grant);
            view.sensitive = true;
            view.pending_approval = true;
            view.transient = true;
            self.directories.push(view);
        }
        self.data_root = state.local_data.root().to_string_lossy().into_owned();
        self.consent_path = grant.host_path.to_string_lossy().into_owned();
        self.consent_request = request;
        self.pending_directory = grant.form_value();
        self.consent_existing = existing;
        self
    }

    pub(super) fn open_settings(mut self) -> Self {
        self.settings_open = true;
        self
    }

    pub(super) fn open_directories(mut self) -> Self {
        self.directories_open = true;
        self
    }

    pub(super) fn with_settings_fields(
        mut self,
        state: &crate::state::AppState,
        fields: SubmittedSettingsFields<'_>,
    ) -> Self {
        self.model_picker = ModelPicker::new(
            &state.vault,
            &state.preferences,
            &state.models_dev,
            fields.provider,
            fields.model,
            fields.thinking,
        );
        self.instructions = fields.instructions;
        self.tool_options = tool_options(&fields.tools);
        let selected_environment = EnvironmentId::parse(fields.environment);
        self.environment_options = environment_options(
            &state.environments,
            &state.environment_snapshots,
            selected_environment,
        );
        self.environment_summary = environment_summary(
            &state.environments,
            &state.environment_snapshots,
            selected_environment,
        );
        self.network_options = network_options(fields.network);
        self.network_domains = fields.network_domains.to_owned();
        self.network_summary = network_summary_from_form(fields.network);
        self.settings_open = true;
        self
    }

    pub(super) fn with_workflow_progress(
        mut self,
        workflow_progress: Option<WorkflowProgressView>,
    ) -> Self {
        if let ConversationPageState::Saved(saved) = &mut self.state {
            saved.workflow_progress = workflow_progress;
        }
        self
    }

    pub(super) fn contents(&self) -> impl Template + '_ {
        self.as_conversation_detail()
    }
}

fn directory_views(grants: &[crate::execution::DirectoryGrant]) -> Vec<DirectoryView> {
    let mut views = grants.iter().map(directory_view).collect::<Vec<_>>();
    let names = views
        .iter()
        .map(|view| view.name.clone())
        .collect::<Vec<_>>();
    for (index, view) in views.iter_mut().enumerate() {
        if names
            .iter()
            .enumerate()
            .any(|(other, name)| other != index && name == &names[index])
        {
            view.name = format!("{} ({})", view.name, view.alias);
        }
    }
    views
}

fn directory_view(grant: &crate::execution::DirectoryGrant) -> DirectoryView {
    DirectoryView {
        id: grant.id.as_hex(),
        name: grant
            .host_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| grant.host_path.to_string_lossy().into_owned()),
        alias: grant.alias.clone(),
        host_path: grant.host_path.to_string_lossy().into_owned(),
        guest_path: grant.guest_path(),
        form_value: grant.form_value(),
        available: grant.is_available(),
        sensitive: false,
        pending_approval: false,
        transient: false,
    }
}

fn environment_options(
    catalogue: &EnvironmentCatalogue,
    snapshots: &EnvironmentSnapshotRepository,
    selected: Option<EnvironmentId>,
) -> Vec<EnvironmentOption> {
    let records = catalogue.list();
    let mut options = records
        .iter()
        .map(|record| {
            let (readiness, availability) = environment_status(record, catalogue, snapshots);
            EnvironmentOption {
                id: record.id.as_hex(),
                name: record.name.clone(),
                readiness,
                availability,
                selected: selected == Some(record.id),
            }
        })
        .collect::<Vec<_>>();
    if let Some(id) = selected
        && records.iter().all(|record| record.id != id)
    {
        options.push(EnvironmentOption {
            id: id.as_hex(),
            name: "Selected environment unavailable".to_owned(),
            readiness: "Unavailable",
            availability: "Unavailable",
            selected: true,
        });
    }
    options
}

fn environment_summary(
    catalogue: &EnvironmentCatalogue,
    snapshots: &EnvironmentSnapshotRepository,
    selected: Option<EnvironmentId>,
) -> String {
    let Some(selected) = selected else {
        return "Choose environment".to_owned();
    };
    let Some(record) = catalogue.get(&selected) else {
        return "Environment unavailable".to_owned();
    };
    let (readiness, availability) = environment_status(&record, catalogue, snapshots);
    format!("{} · {readiness} · {availability}", record.name)
}

fn environment_status(
    record: &crate::environments::EnvironmentRecord,
    catalogue: &EnvironmentCatalogue,
    snapshots: &EnvironmentSnapshotRepository,
) -> (&'static str, &'static str) {
    let latest = catalogue.preparation(&record.latest_preparation);
    let readiness = match latest.as_ref().map(|preparation| preparation.state) {
        Some(PreparationState::Ready) => "Ready",
        Some(PreparationState::Queued) => "Queued",
        Some(PreparationState::Preparing) => "Preparing",
        Some(PreparationState::Failed) => "Preparation failed",
        Some(PreparationState::Interrupted) => "Preparation interrupted",
        Some(PreparationState::Cancelled) => "Preparation cancelled",
        Some(PreparationState::Superseded) => "Preparation superseded",
        None => "Not ready",
    };
    let availability = record
        .ready_preparation
        .and_then(|id| catalogue.preparation(&id))
        .and_then(|preparation| preparation.snapshot)
        .map(|snapshot| snapshots.recorded_availability(&snapshot))
        .map_or("No snapshot", |availability| match availability {
            SnapshotAvailability::Available => "Available",
            SnapshotAvailability::Missing => "Snapshot unavailable",
            SnapshotAvailability::Corrupt => "Snapshot corrupt",
        });
    (readiness, availability)
}

fn network_options(selected: &str) -> Vec<NetworkOption> {
    vec![
        NetworkOption {
            value: "none",
            label: "Off",
            selected: selected.is_empty() || selected == "none",
        },
        NetworkOption {
            value: "restricted",
            label: "Restricted domains",
            selected: selected == "restricted",
        },
        NetworkOption {
            value: "public",
            label: "Public internet",
            selected: selected == "public",
        },
    ]
}

fn network_summary_from_form(network: &str) -> String {
    match network {
        "restricted" => "Restricted domains".to_owned(),
        "public" => "Public internet".to_owned(),
        _ => "Network off".to_owned(),
    }
}

fn tool_options(selected: &[String]) -> Vec<ToolOption> {
    ToolId::ALL
        .into_iter()
        .map(|tool| ToolOption {
            field_name: match tool {
                ToolId::List => "tool_list",
                ToolId::Read => "tool_read",
                ToolId::Write => "tool_write",
                ToolId::Run => "tool_run",
            },
            value: tool.as_str(),
            label: tool.label(),
            detail: match tool {
                ToolId::List => "List files in private scratch storage or authorised directories.",
                ToolId::Read => "Read files in private scratch storage or authorised directories.",
                ToolId::Write => "Write files in private scratch storage or a candidate workspace.",
                ToolId::Run => "Run commands in the sandbox.",
            },
            selected: selected.iter().any(|value| value == tool.as_str()),
        })
        .collect()
}

pub(super) fn workflow_progress(run: &WorkflowRun) -> WorkflowProgressView {
    WorkflowProgressView {
        run_href: format!("/runs/{}", run.id.as_hex()),
        name: run.pinned.definition.name().to_owned(),
        state: run.state.as_label(),
        current_step: run
            .current_step_name()
            .map(str::to_owned)
            .unwrap_or_else(|| "Finished".to_owned()),
        result: workflow_result_label(&run.state),
        task_progress: String::new(),
        loop_id: String::new(),
        command_token: String::new(),
        can_pause: false,
        can_continue: false,
        can_retry: false,
        can_stop: false,
        pause_requested: false,
        awaiting_gate: false,
    }
}

pub(super) fn loop_progress(
    record: &crate::workflows::TaskLoop,
    awaiting_gate: bool,
) -> WorkflowProgressView {
    WorkflowProgressView {
        run_href: format!("/runs/loops/{}", record.id.as_hex()),
        name: record.pinned.definition.name().to_owned(),
        state: record.state.as_label(),
        current_step: record.progress_label(),
        result: loop_result_label(&record.state),
        task_progress: record.progress_label(),
        loop_id: record.id.as_hex(),
        command_token: record.command_token(),
        can_pause: matches!(
            record.state,
            crate::workflows::task_loop::TaskLoopState::Active { .. }
                | crate::workflows::task_loop::TaskLoopState::AwaitingChild { .. }
        ),
        can_continue: record.allows_continue(),
        can_retry: record.allows_retry(),
        can_stop: !record.state.is_terminal(),
        pause_requested: record.pause_requested(),
        awaiting_gate,
    }
}

fn loop_result_label(state: &crate::workflows::task_loop::TaskLoopState) -> &'static str {
    match state {
        crate::workflows::task_loop::TaskLoopState::Completed => {
            "Each completed task kept its commit. Open the parent run for task progress."
        }
        crate::workflows::task_loop::TaskLoopState::Blocked => {
            "The task loop is unavailable until reconciliation finishes. Earlier commits remain."
        }
        crate::workflows::task_loop::TaskLoopState::Cancelled
        | crate::workflows::task_loop::TaskLoopState::Stopped => {
            "The task loop stopped. Earlier commits remain. This did not roll back the project."
        }
        crate::workflows::task_loop::TaskLoopState::Interrupted
        | crate::workflows::task_loop::TaskLoopState::Failed => {
            "The current task stopped. Retry starts a fresh attempt from the recorded task base. It does not resume the earlier transcript."
        }
        crate::workflows::task_loop::TaskLoopState::AwaitingChild { .. } => {
            "A task waits for a human decision. The conversation stays reserved."
        }
        crate::workflows::task_loop::TaskLoopState::PauseRequested { .. } => {
            "A pause waits until this task finishes review and commit. Pause is not approval."
        }
        crate::workflows::task_loop::TaskLoopState::Paused => {
            "The loop is paused after a completed task. Continue starts the next pending task."
        }
        _ => "Each task uses a fresh worker context. Earlier worker transcripts stay excluded.",
    }
}

fn workflow_result_label(state: &crate::workflows::run::RunState) -> &'static str {
    match state {
        crate::workflows::run::RunState::Completed => {
            "The terminal result and detailed worker evidence stay in the run record."
        }
        crate::workflows::run::RunState::Failed
        | crate::workflows::run::RunState::Escalated { .. } => {
            "The run stopped. Open the run record for its terminal result and retained evidence."
        }
        crate::workflows::run::RunState::Cancelled => {
            "The run was cancelled. Earlier evidence stays in the run record."
        }
        _ => "Worker activity stays in the run record and does not enter this conversation.",
    }
}

pub(super) fn pending_code_gate(
    run: &WorkflowRun,
    store: &crate::workflows::WorkflowArtefactRepository,
) -> Option<PendingCodeGateView> {
    let gate = run
        .gates
        .iter()
        .rev()
        .find(|gate| gate.state == crate::workflows::gates::HumanGateState::AwaitingDecision)?;
    let diff = crate::workflows::artefacts::CandidateDiff::load(
        run,
        &gate.diff_base,
        &gate.candidate,
        store,
    )
    .ok()?;
    let (_, changes) = diff.manifest_page(0, 16).ok()?;
    Some(PendingCodeGateView {
        run_id: run.id.as_hex(),
        gate_id: gate.id.as_hex(),
        revision: gate.revision.get().to_string(),
        candidate: diff.target.as_str().to_owned(),
        diff_base: diff.base.as_str().to_owned(),
        diff_href: format!("/runs/{}/gates/{}", run.id.as_hex(), gate.id.as_hex()),
        review_href: format!(
            "/conversations/candidate-review?run={}&candidate={}&diff_base={}",
            run.id.as_hex(),
            gate.candidate.id.as_hex(),
            gate.diff_base.id.as_hex()
        ),
        changes: changes
            .into_iter()
            .map(|change| CandidateChangeView {
                path: change.path,
                status: change.status,
            })
            .collect(),
    })
}

// The transcript leaves envelope space for controls and retains stable message indices.
fn visible_messages(record: &ConversationRecord, byte_budget: usize) -> Vec<MessageView> {
    let mut messages = Vec::new();
    let mut bytes = 0;
    for (index, message) in record.messages.iter().enumerate().rev() {
        let mut view = message_view(index, message);
        if view.saveable_plan {
            view.plan_action = format!("/conversations/{}/plans", record.id.as_hex());
            view.task_action = format!("/conversations/{}/tasks", record.id.as_hex());
            view.conversation_revision = record.revision.to_string();
        }
        bytes += view.html.len() + 2048;
        if bytes > byte_budget || messages.len() >= 64 {
            break;
        }
        messages.push(view);
    }
    messages.reverse();
    messages
}

fn message_view(index: usize, message: &ConversationMessage) -> MessageView {
    let user = message.role == MessageRole::User;
    MessageView {
        index,
        user,
        html: if user {
            format!(
                "<p class=\"whitespace-pre-wrap\">{}</p>",
                ammonia::clean_text(&message.text)
            )
        } else {
            reply_html(&message.text)
        },
        status: match message.status {
            MessageStatus::Complete => "",
            MessageStatus::Pending => "Replying",
            MessageStatus::Interrupted => "Interrupted",
            MessageStatus::Failed => "Failed",
        },
        error: message_error(message),
        streaming: message.status == MessageStatus::Pending,
        saveable_plan: !user
            && message.status == MessageStatus::Complete
            && !message.text.trim().is_empty(),
        task_title: format!("Tasks from response {}", index + 1),
        task_action: String::new(),
        plan_title: format!("Plan from response {}", index + 1),
        plan_action: String::new(),
        conversation_revision: String::new(),
    }
}

pub(super) fn message_error(message: &ConversationMessage) -> String {
    if message.status == MessageStatus::Failed {
        message.error.clone().unwrap_or_else(|| {
            "The reply failed. No error details are available for this message.".to_owned()
        })
    } else {
        String::new()
    }
}

fn plan_document_view(document: &PlanDocument) -> PlanDocumentView {
    let revision = document.current();
    PlanDocumentView {
        title: document.title.clone(),
        prepare_href: document
            .associated_conversation
            .map_or_else(String::new, |id| {
                format!("/conversations/{id}/plans/{}/tasks", document.id)
            }),
        kind: document.kind.label().to_owned(),
        task_list: document.kind == crate::conversations::DocumentKind::TaskList,
        revision: revision.revision,
        provenance: source_label(&revision.source),
        content_hash: revision.content_hash.as_str(),
        open_href: format!("/plans/{}", document.id.as_hex()),
        export_href: format!("/plans/{}/export", document.id.as_hex()),
        review_href: format!(
            "/conversations/{}/plans/{}/review?revision={}",
            document
                .associated_conversation
                .expect("associated plan document")
                .as_hex(),
            document.id.as_hex(),
            revision.revision
        ),
        remove_href: format!(
            "/conversations/{}/plans/{}/remove",
            document
                .associated_conversation
                .expect("associated plan document")
                .as_hex(),
            document.id.as_hex()
        ),
    }
}

fn source_label(source: &PlanSource) -> String {
    match source {
        PlanSource::ConversationMessage { message_index, .. } => {
            format!("Assistant message {}", message_index + 1)
        }
        PlanSource::SubmittedText { .. } => "Submitted document text".to_owned(),
        PlanSource::ProjectFile {
            project_id, path, ..
        } => format!("Project {project_id}: {path}"),
        PlanSource::Correction { previous } => {
            format!("Correction of revision {}", previous.revision)
        }
    }
}

pub(super) struct TaskListItemView {
    pub(super) index: u32,
    pub(super) checked: bool,
    pub(super) markdown: String,
    pub(super) run_href: String,
}

pub(super) struct PlanPageRevision {
    pub(super) revision: u32,
    pub(super) provenance: String,
    pub(super) content_hash: String,
    pub(super) open_href: String,
    pub(super) export_href: String,
}

#[derive(Template)]
#[template(path = "conversations/templates/plan.html", block = "plan_page")]
pub(super) struct PlanDocumentPage {
    pub(super) document_title: String,
    pub(super) title: String,
    pub(super) document_id: String,
    pub(super) document_revision: u32,
    pub(super) current_revision: u32,
    pub(super) provenance: String,
    pub(super) content_hash: String,
    pub(super) content: String,
    pub(super) content_html: String,
    pub(super) revisions: Vec<PlanPageRevision>,
    pub(super) back_href: String,
    pub(super) associated: bool,
    pub(super) task_count: usize,
    pub(super) eligible_task_count: usize,
    pub(super) tasks: Vec<TaskListItemView>,
    pub(super) error: &'static str,
}

#[derive(Template)]
#[template(path = "conversations/templates/plan.html", block = "plan_detail")]
pub(super) struct PlanDocumentContents<'a> {
    pub(super) title: &'a str,
    pub(super) document_id: &'a str,
    pub(super) document_revision: u32,
    pub(super) current_revision: u32,
    pub(super) provenance: &'a str,
    pub(super) content_hash: &'a str,
    pub(super) content: &'a str,
    pub(super) content_html: &'a str,
    pub(super) revisions: &'a [PlanPageRevision],
    pub(super) back_href: &'a str,
    pub(super) associated: bool,
    pub(super) task_count: usize,
    pub(super) eligible_task_count: usize,
    pub(super) tasks: &'a [TaskListItemView],
    pub(super) error: &'static str,
}

impl PlanDocumentPage {
    pub(super) fn from_document(
        document: &PlanDocument,
        revision: u32,
        content: String,
        error: &'static str,
    ) -> Self {
        let selected = document
            .revision(revision)
            .unwrap_or_else(|| document.current());
        let revisions = document
            .revisions
            .iter()
            .rev()
            .map(|item| PlanPageRevision {
                revision: item.revision,
                provenance: source_label(&item.source),
                content_hash: item.content_hash.as_str(),
                open_href: format!("/plans/{}?revision={}", document.id.as_hex(), item.revision),
                export_href: format!(
                    "/plans/{}/export?revision={}",
                    document.id.as_hex(),
                    item.revision
                ),
            })
            .collect();
        let associated = document.associated_conversation.is_some();
        let task_list = (document.kind == crate::conversations::DocumentKind::TaskList)
            .then(|| crate::workflows::task_list::parse(&content).ok())
            .flatten();
        let task_count = task_list.as_ref().map_or(0, |list| list.tasks.len());
        let eligible_task_count = task_list
            .as_ref()
            .map_or(0, |list| list.eligible_tasks().count());
        let back_href = document.associated_conversation.map_or_else(
            || "/conversations".to_owned(),
            |id| format!("/conversations/{id}"),
        );
        Self {
            document_title: format!(
                "{} | {} | Power Plant",
                document.title,
                document.kind.label()
            ),
            title: document.title.clone(),
            document_id: document.id.as_hex(),
            document_revision: selected.revision,
            current_revision: document.current_revision(),
            provenance: source_label(&selected.source),
            content_hash: selected.content_hash.as_str(),
            content_html: {
                let preview = task_list
                    .as_ref()
                    .map_or(content.as_str(), |list| list.preamble.as_str());
                let html = reply_html(preview);
                if html.len() > 400 * 1024 {
                    plain_html(preview)
                } else {
                    html
                }
            },
            content,
            revisions,
            back_href,
            associated,
            task_count,
            eligible_task_count,
            tasks: task_list.map_or_else(Vec::new, |list| {
                list.tasks
                    .into_iter()
                    .map(|task| TaskListItemView {
                        index: task.index,
                        checked: task.checked,
                        markdown: task.markdown,
                        run_href: document.associated_conversation.map_or_else(String::new, |conversation| format!(
                            "/conversations/{conversation}/workflow?task_document={}&task_revision={}&task_hash={}&task_index={}",
                            document.id.as_hex(), selected.revision, selected.content_hash.as_str(), task.index
                        )),
                    })
                    .collect()
            }),
            error,
        }
    }

    pub(super) fn contents(&self) -> PlanDocumentContents<'_> {
        PlanDocumentContents {
            title: &self.title,
            document_id: &self.document_id,
            document_revision: self.document_revision,
            current_revision: self.current_revision,
            provenance: &self.provenance,
            content_hash: &self.content_hash,
            content: &self.content,
            content_html: &self.content_html,
            revisions: &self.revisions,
            back_href: &self.back_href,
            associated: self.associated,
            task_count: self.task_count,
            eligible_task_count: self.eligible_task_count,
            tasks: &self.tasks,
            error: self.error,
        }
    }
}

fn plain_html(text: &str) -> String {
    format!("<pre>{}</pre>", ammonia::clean_text(text))
}

pub(super) fn reply_html(text: &str) -> String {
    let html = crate::markdown::render(text);
    if html.len() > 768 * 1024 || html.matches('<').count() > 64 {
        // Dense markup uses plain text so one message cannot exhaust the browser node bound.
        plain_html(text)
    } else {
        html
    }
}

fn format_network_summary(selected: &NetworkAccess, effective: &NetworkAccess) -> String {
    if selected == effective {
        format!("Effective network: {}", network_label(effective))
    } else {
        format!(
            "Selected network: {} · Effective with preset ceiling: {}",
            network_label(selected),
            network_label(effective)
        )
    }
}

fn network_label(access: &NetworkAccess) -> String {
    match access {
        NetworkAccess::None => "No network".to_owned(),
        NetworkAccess::Restricted(domains) => {
            format!("Restricted domains: {}", domains.join(", "))
        }
        NetworkAccess::Public => "Public internet".to_owned(),
    }
}

#[derive(Template)]
#[template(path = "conversations/templates/message_article.html")]
pub(super) struct MessageArticle<'a> {
    pub(super) message: &'a MessageView,
}

#[derive(Template)]
#[template(path = "conversations/templates/message_body.html")]
pub(super) struct MessageBody<'a> {
    pub(super) message: &'a MessageView,
}

#[derive(Template)]
#[template(path = "conversations/templates/observe.html")]
pub(super) struct ConversationObserveContents<'a> {
    pub(super) id: &'a str,
    pub(super) job_id: &'a str,
    pub(super) cursor: u64,
    pub(super) active: bool,
}
