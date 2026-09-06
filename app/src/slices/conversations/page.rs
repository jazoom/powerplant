use askama::Template;

#[cfg(test)]
mod tests;

use crate::{
    agents::AgentRecord,
    conversations::{
        ConversationMessage, ConversationModelConfiguration, ConversationRecord, MessageRole,
        MessageStatus,
    },
    models::models_dev::ModelsDevCatalogue,
    providers::ModelSelection,
    sessions::{JobSnapshot, JobStatus},
    vault::ProviderVault,
};

pub(super) const CATALOGUE_TITLE: &str = "Conversations | Power Plant";
pub(super) const NEW_TITLE: &str = "New conversation | Power Plant";

pub(super) struct ConversationListItem {
    pub(super) id: String,
    pub(super) title: String,
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "conversation_catalogue"
)]
pub(super) struct CatalogueView {
    pub(super) conversations: Vec<ConversationListItem>,
}

impl CatalogueView {
    pub(super) fn from_records(records: &[ConversationRecord]) -> Self {
        let mut conversations: Vec<_> = records
            .iter()
            .map(|record| ConversationListItem {
                id: record.id.as_hex(),
                title: record.title.clone(),
            })
            .collect();
        conversations.sort_by(|left, right| left.title.cmp(&right.title));
        Self { conversations }
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
}

pub(super) struct ModelSources<'a> {
    pub(super) vault: &'a ProviderVault,
    pub(super) models: &'a ModelsDevCatalogue,
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
    pub(super) model_summary: &'a str,
    pub(super) model_available: bool,
    pub(super) job_id: &'a str,
    pub(super) cursor: u64,
    pub(super) job_active: bool,
    pub(super) session_busy: bool,
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
    pub(super) model_summary: String,
    pub(super) model_available: bool,
    pub(super) job_id: String,
    pub(super) cursor: u64,
    pub(super) job_active: bool,
    pub(super) session_busy: bool,
}

impl ConversationDetailView {
    pub(super) fn from_record(
        record: &ConversationRecord,
        sources: ModelSources<'_>,
        agents: &[AgentRecord],
        job: Option<&JobSnapshot>,
        session_busy: bool,
        title: &str,
        error: &'static str,
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
        let messages = visible_messages(record);
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
            model_summary,
            job_id,
            cursor,
            job_active,
            session_busy,
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
            model_summary: &self.model_summary,
            model_available: self.model_available,
            job_id: &self.job_id,
            cursor: self.cursor,
            job_active: self.job_active,
            session_busy: self.session_busy,
        }
    }
}

// The transcript leaves envelope space for controls and retains stable message indices.
fn visible_messages(record: &ConversationRecord) -> Vec<MessageView> {
    let mut messages = Vec::new();
    let mut bytes = 0;
    for (index, message) in record.messages.iter().enumerate().rev() {
        let view = message_view(index, message);
        bytes += view.html.len() + 1024;
        if bytes > 800 * 1024 || messages.len() >= 64 {
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
