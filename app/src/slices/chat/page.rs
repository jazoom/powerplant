use askama::Template;

use crate::{
    agents::AgentRecord,
    markdown,
    models::ModelCatalogue,
    providers::{ChatTurn, Role},
    sessions::{JobSnapshot, JobStatus, SessionSnapshot},
    vault::{DeskProvider, ProviderVault},
};

pub(super) const DOCUMENT_TITLE: &str = "Chat | Power Plant";

pub(super) struct TurnView {
    pub(super) id: String,
    pub(super) is_user: bool,
    pub(super) html: String,
}

pub(super) struct DeskProviderOption {
    pub(super) value: &'static str,
    pub(super) label: &'static str,
    pub(super) model: String,
    pub(super) selected: bool,
}

pub(super) struct ModelOption {
    pub(super) id: String,
    pub(super) selected: bool,
}

#[derive(Template)]
#[template(path = "chat/templates/chat.html")]
pub(super) struct ChatViewModel {
    pub(super) providers: Vec<DeskProviderOption>,
    pub(super) model: String,
    pub(super) favourite_models: Vec<ModelOption>,
    pub(super) catalogue_models: Vec<ModelOption>,
    pub(super) catalogue_pending: bool,
    pub(super) desk_error: &'static str,
    pub(super) turns: Vec<TurnView>,
    pub(super) error: &'static str,
    pub(super) job_error: &'static str,
    pub(super) job_id: String,
    pub(super) cursor: u64,
    pub(super) job_active: bool,
    pub(super) job_status: &'static str,
    pub(super) session_busy: bool,
    pub(super) agent_id: String,
    pub(super) agent_name: String,
    pub(super) run_id: String,
    pub(super) run_step: String,
    pub(super) workflow_name: String,
    pub(super) workflow_options: Vec<WorkflowOption>,
    pub(super) workflow_empty: bool,
    pub(super) draft_message: String,
    pub(super) environment_preview: Vec<PreviewLine>,
    pub(super) environment_preview_error: &'static str,
    pub(super) preview_ready: bool,
}

pub(super) struct PreviewLine {
    pub(super) text: String,
}

pub(super) struct WorkflowOption {
    pub(super) token: String,
    pub(super) label: String,
    pub(super) selected: bool,
}

impl ChatViewModel {
    pub(super) fn from_session(
        record: &AgentRecord,
        session: &SessionSnapshot,
        vault: &ProviderVault,
        catalogue: &ModelCatalogue,
        error: &'static str,
        desk_error: &'static str,
    ) -> Self {
        Self::from_parts(
            record,
            &vault.desk_providers(),
            catalogue,
            &session.turns,
            session.job.as_ref(),
            session.session_busy,
            error,
            desk_error,
        )
    }

    pub(super) fn desk_only(
        vault: &ProviderVault,
        catalogue: &ModelCatalogue,
        session_busy: bool,
        desk_error: &'static str,
    ) -> Self {
        Self::from_parts(
            &AgentRecord {
                id: crate::agents::AgentId::parse(&"0".repeat(32)).expect("zero id"),
                revision: 1,
                name: String::new(),
                instructions: String::new(),
                tools: Vec::new(),
                directories: Vec::new(),
                primary_directory: String::new(),
            },
            &vault.desk_providers(),
            catalogue,
            &[],
            None,
            session_busy,
            "",
            desk_error,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        record: &AgentRecord,
        providers: &[DeskProvider],
        catalogue: &ModelCatalogue,
        turns: &[ChatTurn],
        job: Option<&JobSnapshot>,
        session_busy: bool,
        error: &'static str,
        desk_error: &'static str,
    ) -> Self {
        let mut views: Vec<TurnView> = turns
            .iter()
            .enumerate()
            .map(|(index, turn)| turn_view(index, turn))
            .collect();
        let mut job_id = String::new();
        let mut cursor = 0;
        let mut job_active = false;
        let mut job_status = "";
        let mut run_id = String::new();
        let mut run_step = String::new();
        let mut workflow_name = String::new();
        if let Some(job) = job {
            run_id = job.run_id.as_hex();
            run_step = job.step_label.clone();
            workflow_name = job.workflow_name.clone();
            if !job.output.is_empty() && views.len() == job.assistant_index {
                views.push(assistant_turn(job.assistant_index, &job.output));
            }
            if job.status == JobStatus::Running {
                job_id = job.id.as_hex();
                cursor = job.latest_seq;
                job_active = true;
                job_status = if job.cancel_requested {
                    "Stopping"
                } else if job.step_label == "Preparing environment" {
                    "Preparing environment"
                } else if job.step_label == "Source capture" {
                    "Source capture"
                } else {
                    "Writing"
                };
            }
        }
        let selected = providers.iter().find(|provider| provider.selected);
        let model = selected
            .map(|provider| provider.model.clone())
            .unwrap_or_default();
        let (favourite_models, catalogue_models) = selected
            .map(|provider| {
                model_options(
                    &provider.model,
                    &provider.favourites,
                    &catalogue.list(provider.kind),
                )
            })
            .unwrap_or_default();
        let catalogue_pending = selected.is_some_and(|provider| catalogue.pending(provider.kind));
        Self {
            providers: providers
                .iter()
                .map(|provider| DeskProviderOption {
                    value: provider.kind.as_str(),
                    label: provider.kind.label(),
                    model: provider.model.clone(),
                    selected: provider.selected,
                })
                .collect(),
            model,
            favourite_models,
            catalogue_models,
            catalogue_pending,
            desk_error,
            turns: views,
            error,
            job_error: "",
            job_id,
            cursor,
            job_active,
            job_status,
            session_busy,
            agent_id: record.id.as_hex(),
            agent_name: record.name.clone(),
            run_id,
            run_step,
            workflow_name,
            workflow_options: Vec::new(),
            workflow_empty: false,
            draft_message: String::new(),
            environment_preview: Vec::new(),
            environment_preview_error: "",
            preview_ready: false,
        }
    }

    pub(super) fn desk_settings(&self) -> DeskSettingsContents<'_> {
        DeskSettingsContents {
            providers: &self.providers,
            model: &self.model,
            favourite_models: &self.favourite_models,
            catalogue_models: &self.catalogue_models,
            catalogue_pending: self.catalogue_pending,
            desk_error: self.desk_error,
            job_active: self.session_busy,
        }
    }

    pub(super) fn desk_model_catalogue(&self) -> DeskModelCatalogueContents<'_> {
        DeskModelCatalogueContents {
            favourite_models: &self.favourite_models,
            catalogue_models: &self.catalogue_models,
            catalogue_pending: self.catalogue_pending,
        }
    }

    pub(super) fn composer(&self) -> ComposerContents<'_> {
        ComposerContents {
            error: self.error,
            session_busy: self.session_busy,
            agent_id: &self.agent_id,
            workflow_options: &self.workflow_options,
            workflow_empty: self.workflow_empty,
            draft_message: &self.draft_message,
            environment_preview: &self.environment_preview,
            environment_preview_error: self.environment_preview_error,
            preview_ready: self.preview_ready,
        }
    }

    pub(super) fn job_observe(&self) -> JobObserveContents<'_> {
        self.job_observe_with("")
    }

    pub(super) fn job_observe_with<'a>(&'a self, job_error: &'a str) -> JobObserveContents<'a> {
        JobObserveContents {
            job_error,
            job_id: &self.job_id,
            cursor: self.cursor,
            job_active: self.job_active,
            job_status: self.job_status,
            agent_id: &self.agent_id,
            run_id: &self.run_id,
            run_step: &self.run_step,
            workflow_name: &self.workflow_name,
        }
    }
}

pub(super) fn user_turn(index: usize, text: &str) -> TurnView {
    TurnView {
        id: turn_id(index),
        is_user: true,
        html: format!("<p>{}</p>", markdown::escape_plain(text)),
    }
}

pub(super) fn assistant_turn(index: usize, text: &str) -> TurnView {
    TurnView {
        id: turn_id(index),
        is_user: false,
        html: markdown::render(text),
    }
}

pub(super) fn turn_id(index: usize) -> String {
    format!("turn-{}", index + 1)
}

#[derive(Template)]
#[template(path = "chat/templates/chat.html", block = "desk_settings")]
pub(super) struct DeskSettingsContents<'a> {
    pub(super) providers: &'a [DeskProviderOption],
    pub(super) model: &'a str,
    pub(super) favourite_models: &'a [ModelOption],
    pub(super) catalogue_models: &'a [ModelOption],
    pub(super) catalogue_pending: bool,
    pub(super) desk_error: &'a str,
    pub(super) job_active: bool,
}

#[derive(Template)]
#[template(path = "chat/templates/chat.html", block = "desk_model_catalogue")]
pub(super) struct DeskModelCatalogueContents<'a> {
    pub(super) favourite_models: &'a [ModelOption],
    pub(super) catalogue_models: &'a [ModelOption],
    pub(super) catalogue_pending: bool,
}

#[derive(Template)]
#[template(path = "chat/templates/chat.html", block = "transcript")]
pub(super) struct TranscriptContents<'a> {
    pub(super) turns: &'a [TurnView],
}

#[derive(Template)]
#[template(path = "chat/templates/chat.html", block = "composer")]
pub(super) struct ComposerContents<'a> {
    pub(super) error: &'a str,
    pub(super) session_busy: bool,
    pub(super) agent_id: &'a str,
    pub(super) workflow_options: &'a [WorkflowOption],
    pub(super) workflow_empty: bool,
    pub(super) draft_message: &'a str,
    pub(super) environment_preview: &'a [PreviewLine],
    pub(super) environment_preview_error: &'static str,
    pub(super) preview_ready: bool,
}

#[derive(Template)]
#[template(path = "chat/templates/chat.html", block = "job_observe")]
pub(super) struct JobObserveContents<'a> {
    pub(super) job_error: &'a str,
    pub(super) job_id: &'a str,
    pub(super) cursor: u64,
    pub(super) job_active: bool,
    pub(super) job_status: &'static str,
    pub(super) agent_id: &'a str,
    pub(super) run_id: &'a str,
    pub(super) run_step: &'a str,
    pub(super) workflow_name: &'a str,
}

impl<'a> JobObserveContents<'a> {
    pub(super) fn idle(
        error: &'a str,
        agent_id: &'a str,
        run_id: &'a str,
        run_step: &'a str,
        workflow_name: &'a str,
    ) -> Self {
        Self {
            job_error: error,
            job_id: "",
            cursor: 0,
            job_active: false,
            job_status: "",
            agent_id,
            run_id,
            run_step,
            workflow_name,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn observing(
        job_id: &'a str,
        cursor: u64,
        status: &'static str,
        error: &'a str,
        agent_id: &'a str,
        run_id: &'a str,
        run_step: &'a str,
        workflow_name: &'a str,
    ) -> Self {
        Self {
            job_error: error,
            job_id,
            cursor,
            job_active: true,
            job_status: status,
            agent_id,
            run_id,
            run_step,
            workflow_name,
        }
    }
}

#[derive(Template)]
#[template(path = "chat/templates/job_cursor.html")]
pub(super) struct JobCursorContents<'a> {
    pub(super) job_id: &'a str,
    pub(super) cursor: u64,
}

#[derive(Template)]
#[template(path = "chat/templates/turn_article.html")]
pub(super) struct TurnArticle<'a> {
    pub(super) turn: &'a TurnView,
}

#[derive(Template)]
#[template(path = "chat/templates/turn_body.html")]
pub(super) struct TurnBody<'a> {
    pub(super) turn: &'a TurnView,
}

fn model_options(
    current: &str,
    favourites: &[String],
    listed: &[String],
) -> (Vec<ModelOption>, Vec<ModelOption>) {
    let current_favourite = favourites.iter().any(|item| item == current);
    let favourite_models = favourites
        .iter()
        .map(|id| ModelOption {
            id: id.clone(),
            selected: id == current,
        })
        .collect();
    let mut catalogue_models: Vec<ModelOption> = listed
        .iter()
        .filter(|id| !favourites.contains(id))
        .map(|id| ModelOption {
            id: id.clone(),
            selected: id == current,
        })
        .collect();
    if !current.is_empty() && !current_favourite && !listed.iter().any(|id| id == current) {
        catalogue_models.insert(
            0,
            ModelOption {
                id: current.to_owned(),
                selected: true,
            },
        );
    }
    (favourite_models, catalogue_models)
}

fn turn_view(index: usize, turn: &ChatTurn) -> TurnView {
    match turn.role {
        Role::User => user_turn(index, &turn.text),
        Role::Assistant => assistant_turn(index, &turn.text),
    }
}
