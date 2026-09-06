use askama::Template;

#[cfg(test)]
mod tests;

use crate::{
    agents::{AgentRecord, NetworkAccess},
    conversations::{
        ConversationMessage, ConversationModelConfiguration, ConversationRecord,
        MAXIMUM_PROJECT_ASSOCIATIONS, MessageRole, MessageStatus, PlanDocument, PlanSource,
    },
    models::models_dev::ModelsDevCatalogue,
    projects::{ProjectId, ProjectRecord},
    providers::ModelSelection,
    sessions::{JobSnapshot, JobStatus},
    vault::ProviderVault,
    workflows::WorkflowRun,
};

pub(super) const CATALOGUE_TITLE: &str = "Conversations | Power Plant";
pub(super) const NEW_TITLE: &str = "New conversation | Power Plant";

pub(super) struct ConversationListItem {
    pub(super) id: String,
    pub(super) title: String,
}

pub(super) struct CatalogueProjectOption {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) selected: bool,
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

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "conversation_form"
)]
pub(super) struct ConversationFormContents<'a> {
    pub(super) title: &'a str,
    pub(super) error: &'static str,
}

#[derive(Template)]
#[template(path = "conversations/templates/index.html", block = "new_page")]
pub(super) struct ConversationFormView {
    pub(super) title: String,
    pub(super) error: &'static str,
}

impl ConversationFormView {
    pub(super) fn new(title: &str, error: &'static str) -> Self {
        Self {
            title: title.to_owned(),
            error,
        }
    }

    pub(super) fn contents(&self) -> ConversationFormContents<'_> {
        ConversationFormContents {
            title: &self.title,
            error: self.error,
        }
    }
}

pub(super) struct MessageView {
    pub(super) index: usize,
    pub(super) user: bool,
    pub(super) html: String,
    pub(super) status: &'static str,
    pub(super) streaming: bool,
    pub(super) saveable_plan: bool,
    pub(super) plan_title: String,
    pub(super) plan_action: String,
    pub(super) conversation_revision: String,
}

pub(super) struct PlanDocumentView {
    pub(super) title: String,
    pub(super) revision: u32,
    pub(super) provenance: String,
    pub(super) content_hash: String,
    pub(super) open_href: String,
    pub(super) export_href: String,
    pub(super) remove_href: String,
}

pub(super) struct ModelSources<'a> {
    pub(super) vault: &'a ProviderVault,
    pub(super) models: &'a ModelsDevCatalogue,
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

pub(super) struct NetworkOption {
    pub(super) value: &'static str,
    pub(super) label: &'static str,
    pub(super) selected: bool,
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "conversation_detail"
)]
pub(super) struct ConversationDetailContents<'a> {
    pub(super) heading: &'a str,
    pub(super) title: &'a str,
    pub(super) id: &'a str,
    pub(super) revision: &'a str,
    pub(super) error: &'static str,
    pub(super) messages: &'a [MessageView],
    pub(super) omitted_messages: usize,
    pub(super) providers: &'a [ProviderOption],
    pub(super) presets: &'a [PresetOption],
    pub(super) attached_projects: &'a [ProjectContextView],
    pub(super) attachable_projects: &'a [CatalogueProjectOption],
    pub(super) project_limit_reached: bool,
    pub(super) model_summary: &'a str,
    pub(super) model_available: bool,
    pub(super) job_id: &'a str,
    pub(super) cursor: u64,
    pub(super) job_active: bool,
    pub(super) session_busy: bool,
    pub(super) pending_gate: Option<&'a PendingCodeGateView>,
    pub(super) network_options: &'a [NetworkOption],
    pub(super) network_domains: &'a str,
    pub(super) network_summary: &'a str,
    pub(super) plans: &'a [PlanDocumentView],
}

#[derive(Template)]
#[template(path = "conversations/templates/index.html", block = "detail_page")]
pub(super) struct ConversationDetailView {
    pub(super) heading: String,
    pub(super) document_title: String,
    pub(super) title: String,
    pub(super) id: String,
    pub(super) revision: String,
    pub(super) error: &'static str,
    pub(super) messages: Vec<MessageView>,
    pub(super) omitted_messages: usize,
    pub(super) providers: Vec<ProviderOption>,
    pub(super) presets: Vec<PresetOption>,
    pub(super) attached_projects: Vec<ProjectContextView>,
    pub(super) attachable_projects: Vec<CatalogueProjectOption>,
    pub(super) project_limit_reached: bool,
    pub(super) model_summary: String,
    pub(super) model_available: bool,
    pub(super) job_id: String,
    pub(super) cursor: u64,
    pub(super) job_active: bool,
    pub(super) session_busy: bool,
    pub(super) pending_gate: Option<PendingCodeGateView>,
    pub(super) network_options: Vec<NetworkOption>,
    pub(super) network_domains: String,
    pub(super) network_summary: String,
    pub(super) plans: Vec<PlanDocumentView>,
}
impl ConversationDetailView {
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
    ) -> Self {
        let fallback = sources
            .vault
            .desk_providers()
            .into_iter()
            .find(|provider| provider.selected)
            .map(|connection| {
                ConversationModelConfiguration::direct(ModelSelection {
                    provider: connection.kind,
                    thinking: sources.models.effective_effort(
                        connection.kind,
                        &connection.model,
                        connection.thinking.as_ref(),
                    ),
                    model: connection.model,
                })
            });
        let configuration = record.model.as_ref().or(fallback.as_ref());
        let selection = configuration.map(|configuration| &configuration.selection);
        let network_options = vec![
            NetworkOption {
                value: "none",
                label: "No network",
                selected: record.network == NetworkAccess::None,
            },
            NetworkOption {
                value: "restricted",
                label: "Restricted domains",
                selected: matches!(record.network, NetworkAccess::Restricted(_)),
            },
            NetworkOption {
                value: "public",
                label: "Public internet",
                selected: record.network == NetworkAccess::Public,
            },
        ];
        let network_domains = record.network.domains().join("\n");
        let preset_network = configuration
            .and_then(|configuration| configuration.preset.as_ref())
            .and_then(|selected| agents.iter().find(|agent| agent.id == selected.id))
            .map(|agent| &agent.network);
        let effective_network =
            crate::conversations::intersect_network(&record.network, preset_network);
        let network_summary = format_network_summary(&record.network, &effective_network);
        let providers = sources
            .vault
            .desk_providers()
            .into_iter()
            .map(|provider| ProviderOption {
                value: provider.kind.as_str(),
                label: provider.kind.label(),
                model: selection
                    .filter(|selection| selection.provider == provider.kind)
                    .map_or(provider.model, |selection| selection.model.clone()),
                thinking: selection
                    .filter(|selection| selection.provider == provider.kind)
                    .and_then(|selection| selection.thinking.as_ref())
                    .map(|value| value.as_str().to_owned())
                    .unwrap_or_default(),
                selected: selection.is_some_and(|selection| selection.provider == provider.kind),
            })
            .collect();
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
                let selection = &configuration.selection;
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
        // Plan controls share the envelope with the transcript.
        let message_budget = (800_usize * 1024).saturating_sub(plans.len() * 3072);
        let messages = visible_messages(record, message_budget);
        let omitted_messages = record.messages.len() - messages.len();
        Self {
            heading: record.title.clone(),
            document_title: format!("{} | Power Plant", record.title),
            title: title.to_owned(),
            id: record.id.as_hex(),
            revision: record.revision.to_string(),
            error,
            messages,
            omitted_messages,
            model_available: selection.is_some(),
            providers,
            presets,
            attached_projects,
            attachable_projects,
            project_limit_reached: record.projects.len() >= MAXIMUM_PROJECT_ASSOCIATIONS,
            model_summary,
            job_id,
            cursor,
            job_active,
            session_busy,
            pending_gate,
            network_options,
            network_domains,
            network_summary,
            plans,
        }
    }

    pub(super) fn contents(&self) -> ConversationDetailContents<'_> {
        ConversationDetailContents {
            heading: &self.heading,
            title: &self.title,
            id: &self.id,
            revision: &self.revision,
            error: self.error,
            messages: &self.messages,
            omitted_messages: self.omitted_messages,
            providers: &self.providers,
            presets: &self.presets,
            attached_projects: &self.attached_projects,
            attachable_projects: &self.attachable_projects,
            project_limit_reached: self.project_limit_reached,
            model_summary: &self.model_summary,
            model_available: self.model_available,
            job_id: &self.job_id,
            cursor: self.cursor,
            job_active: self.job_active,
            session_busy: self.session_busy,
            pending_gate: self.pending_gate.as_ref(),
            network_options: &self.network_options,
            network_domains: &self.network_domains,
            network_summary: &self.network_summary,
            plans: &self.plans,
        }
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
            plain_html(&message.text)
        } else {
            reply_html(&message.text)
        },
        status: match message.status {
            MessageStatus::Complete => "",
            MessageStatus::Pending => "Replying",
            MessageStatus::Interrupted => "Interrupted",
            MessageStatus::Failed => "Failed",
        },
        streaming: message.status == MessageStatus::Pending,
        saveable_plan: !user
            && message.status == MessageStatus::Complete
            && !message.text.trim().is_empty(),
        plan_title: format!("Plan from response {}", index + 1),
        plan_action: String::new(),
        conversation_revision: String::new(),
    }
}

fn plan_document_view(document: &PlanDocument) -> PlanDocumentView {
    let revision = document.current();
    PlanDocumentView {
        title: document.title.clone(),
        revision: revision.revision,
        provenance: source_label(&revision.source),
        content_hash: revision.content_hash.as_str(),
        open_href: format!("/plans/{}", document.id.as_hex()),
        export_href: format!("/plans/{}/export", document.id.as_hex()),
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
        PlanSource::SubmittedText { .. } => "Submitted plan text".to_owned(),
        PlanSource::Correction { previous } => {
            format!("Correction of revision {}", previous.revision)
        }
    }
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
        let back_href = document.associated_conversation.map_or_else(
            || "/conversations".to_owned(),
            |id| format!("/conversations/{id}"),
        );
        Self {
            document_title: format!("{} | Plan | Power Plant", document.title),
            title: document.title.clone(),
            document_id: document.id.as_hex(),
            document_revision: selected.revision,
            current_revision: document.current_revision(),
            provenance: source_label(&selected.source),
            content_hash: selected.content_hash.as_str(),
            content_html: {
                let html = reply_html(&content);
                if html.len() > 400 * 1024 {
                    plain_html(&content)
                } else {
                    html
                }
            },
            content,
            revisions,
            back_href,
            associated,
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
