use askama::Template;

pub(super) struct WorkflowEntry {
    pub(super) token: String,
    pub(super) name: String,
    pub(super) summary: String,
    pub(super) selected: bool,
    pub(super) use_href: String,
}

pub(super) struct PresetEntry {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) selected: bool,
    pub(super) preview_href: String,
}

pub(super) struct DestinationEntry {
    pub(super) title: String,
    pub(super) href: String,
}

#[derive(Template)]
#[template(path = "resources/templates/index.html")]
pub(super) struct ResourcesPage {
    pub(super) back_href: String,
    pub(super) back_label: &'static str,
    pub(super) context_title: String,
    pub(super) destination_id: String,
    pub(super) workflows: Vec<WorkflowEntry>,
    pub(super) workflow_error: &'static str,
    pub(super) selected_workflow: String,
    pub(super) selected_workflow_href: String,
    pub(super) presets: Vec<PresetEntry>,
    pub(super) preset_error: &'static str,
    pub(super) selected_preset: String,
    pub(super) workflow_destinations: Vec<DestinationEntry>,
    pub(super) preset_destinations: Vec<DestinationEntry>,
    pub(super) chooser_heading: &'static str,
    pub(super) has_conversations: bool,
}

impl ResourcesPage {
    pub(super) fn new(
        state: &crate::state::AppState,
        context: Option<crate::conversations::ConversationId>,
        workflow_raw: &str,
        preset_raw: &str,
    ) -> Self {
        let context_record = context.and_then(|id| {
            state
                .conversations
                .get(&id)
                .map(|record| (id, record.title.clone()))
        });
        let (back_href, back_label, context_title, destination_id) = match &context_record {
            Some((id, title)) => (
                format!("/conversations/{}", id.as_hex()),
                "Back to conversation",
                title.clone(),
                id.as_hex(),
            ),
            None => (
                "/conversations".to_owned(),
                "Back to conversations",
                String::new(),
                String::new(),
            ),
        };
        let context_suffix = context_record.as_ref().map_or_else(String::new, |(id, _)| {
            format!("conversation={}", id.as_hex())
        });

        let workflow_key = workflow_raw.trim().to_owned();
        let (selected_token, workflow_error) = if workflow_key.is_empty() {
            (None, "")
        } else {
            match crate::workflows::WorkflowSelection::parse(&workflow_key)
                .and_then(|selection| state.workflows.resolve(&selection).map(|_| selection).ok())
            {
                Some(selection) => (Some(selection.as_token()), ""),
                None => (
                    None,
                    "That workflow is no longer available. Choose another.",
                ),
            }
        };
        let workflows = state
            .workflows
            .list()
            .into_iter()
            .map(|record| {
                let token = crate::workflows::WorkflowSelection {
                    workflow_id: record.id,
                    definition_version: record.definition_version,
                }
                .as_token();
                let selected = selected_token.as_deref() == Some(token.as_str());
                let use_href = match &context_record {
                    Some((id, _)) => {
                        format!("/conversations/{}/workflow?workflow={token}", id.as_hex())
                    }
                    None => format!("/resources?workflow={token}"),
                };
                WorkflowEntry {
                    token,
                    name: state.workflows.display_name(&record),
                    summary: crate::workflows::summary::process_summary(&record.definition),
                    selected,
                    use_href,
                }
            })
            .collect::<Vec<_>>();
        let selected_workflow = selected_token
            .as_deref()
            .and_then(|token| workflows.iter().find(|entry| entry.token == token))
            .map(|entry| entry.name.clone())
            .unwrap_or_default();
        let selected_workflow_token = selected_token.unwrap_or_default();
        let selected_workflow_href = if selected_workflow.is_empty() {
            String::new()
        } else {
            match &context_record {
                Some((id, _)) => format!(
                    "/conversations/{}/workflow?workflow={selected_workflow_token}",
                    id.as_hex()
                ),
                None => format!("/resources?workflow={selected_workflow_token}"),
            }
        };

        let preset_key = preset_raw.trim().to_owned();
        let (selected_id, preset_error) = if preset_key.is_empty() {
            (None, "")
        } else {
            match crate::presets::PresetId::parse(&preset_key).and_then(|id| state.presets.get(&id))
            {
                Some(record) => (Some(record.id.as_hex()), ""),
                None => (None, "That preset is no longer available. Choose another."),
            }
        };
        let presets = state
            .presets
            .list()
            .into_iter()
            .map(|record| {
                let id = record.id.as_hex();
                let selected = selected_id.as_deref() == Some(id.as_str());
                let preview_href = if context_suffix.is_empty() {
                    format!("/resources?preset={id}")
                } else {
                    format!("/resources?{context_suffix}&preset={id}")
                };
                PresetEntry {
                    description: preset_summary(&record.settings),
                    name: record.name,
                    selected,
                    preview_href,
                    id,
                }
            })
            .collect::<Vec<_>>();
        let selected_preset = selected_id
            .as_deref()
            .and_then(|id| presets.iter().find(|entry| entry.id == id))
            .map(|entry| entry.name.clone())
            .unwrap_or_default();
        let selected_preset_id = selected_id.unwrap_or_default();

        // The destination chooser stays on the canonical resource GET.
        // It retains the selected resource and never creates a record,
        // starts a workflow or grants access from navigation. Workflow and
        // preset selections keep separate destinations so a combined query
        // cannot mix their handoffs.
        let mut conversations = state.conversations.list();
        conversations.sort_by_key(|record| std::cmp::Reverse(record.updated_at_ms));
        let has_conversations = !conversations.is_empty();
        let workflow_destinations =
            if !selected_workflow_token.is_empty() && context_record.is_none() {
                let token = selected_workflow_token.clone();
                conversations
                    .iter()
                    .map(|record| DestinationEntry {
                        title: record.title.clone(),
                        href: format!(
                            "/conversations/{}/workflow?workflow={token}",
                            record.id.as_hex()
                        ),
                    })
                    .collect()
            } else {
                Vec::new()
            };
        let preset_destinations = if !selected_preset_id.is_empty() && context_record.is_none() {
            let id = selected_preset_id.clone();
            conversations
                .iter()
                .map(|record| DestinationEntry {
                    href: format!("/resources?conversation={}&preset={id}", record.id.as_hex()),
                    title: record.title.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let chooser_heading = if (!selected_workflow_token.is_empty()
            || !selected_preset_id.is_empty())
            && context_record.is_none()
        {
            "Choose a conversation"
        } else {
            ""
        };

        Self {
            back_href,
            back_label,
            context_title,
            destination_id,
            workflows,
            workflow_error,
            selected_workflow,
            selected_workflow_href,
            presets,
            preset_error,
            selected_preset,
            workflow_destinations,
            preset_destinations,
            chooser_heading,
            has_conversations,
        }
    }
}

fn preset_summary(settings: &crate::execution::ExecutionSettings) -> String {
    let directory_count = settings.directories.len();
    format!(
        "{} · {} · {} tool{} · {} director{}",
        settings.model.provider.label(),
        settings.model.model,
        settings.tools.len(),
        if settings.tools.len() == 1 { "" } else { "s" },
        directory_count,
        if directory_count == 1 { "y" } else { "ies" },
    )
}
