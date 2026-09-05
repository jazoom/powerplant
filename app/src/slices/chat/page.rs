use askama::Template;

use crate::{
    agents::AgentRecord,
    environments::{EnvironmentRecord, PreparationRecord, PreparationState, SnapshotAvailability},
    markdown,
    models::{ModelCatalogue, models_dev::ModelsDevCatalogue},
    projects::ProjectRecord,
    providers::{
        AssistantActivity, AssistantReply, ChatTurn, ProviderKind, Role, ThinkingEffort, ToolOutput,
    },
    sessions::{JobSnapshot, JobStatus, SessionSnapshot},
    vault::{DeskProvider, ProviderVault},
};

pub(crate) const DOCUMENT_TITLE: &str = "Chat | Power Plant";

pub(crate) struct TurnView {
    pub(crate) id: String,
    pub(crate) is_user: bool,
    pub(crate) response_html: String,
    pub(crate) activities: Vec<ActivityView>,
    pub(crate) streaming: bool,
    pub(crate) thinking_active: bool,
}

pub(crate) struct ActivityView {
    pub(crate) is_thinking: bool,
    pub(crate) thinking_html: String,
    pub(crate) tools: Vec<ToolOutputView>,
    pub(crate) thinking_active: bool,
}

pub(crate) struct ToolOutputView {
    pub(crate) label: String,
    pub(crate) output: String,
    pub(crate) preview: String,
}

pub(crate) struct DeskProviderOption {
    pub(crate) value: &'static str,
    pub(crate) label: &'static str,
    pub(crate) model: String,
    pub(crate) thinking: String,
    pub(crate) efforts_json: String,
    pub(crate) selected: bool,
}

pub(crate) struct ThinkingOption {
    pub(crate) value: String,
    pub(crate) label: String,
    pub(crate) selected: bool,
}

pub(crate) struct ModelOption {
    pub(crate) id: String,
    pub(crate) efforts_json: String,
    pub(crate) selected: bool,
}

pub(crate) struct AgentChoice {
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) selected: bool,
}

#[derive(Template)]
#[template(path = "projects/templates/desk.html")]
pub(crate) struct ChatViewModel {
    pub(crate) document_title: String,
    pub(crate) providers: Vec<DeskProviderOption>,
    pub(crate) model: String,
    pub(crate) thinking_options: Vec<ThinkingOption>,
    pub(crate) favourite_models: Vec<ModelOption>,
    pub(crate) catalogue_models: Vec<ModelOption>,
    pub(crate) catalogue_pending: bool,
    pub(crate) desk_error: &'static str,
    pub(crate) turns: Vec<TurnView>,
    pub(crate) error: &'static str,
    pub(crate) job_error: &'static str,
    pub(crate) job_id: String,
    pub(crate) cursor: u64,
    pub(crate) job_active: bool,
    pub(crate) job_status: String,
    pub(crate) session_busy: bool,
    pub(crate) project_id: String,
    pub(crate) project_name: String,
    pub(crate) project_path: String,
    pub(crate) project_available: bool,
    pub(crate) desk_href: String,
    pub(crate) agent_id: String,
    pub(crate) agent_name: String,
    pub(crate) agent_choices: Vec<AgentChoice>,
    pub(crate) show_thinking: bool,
    pub(crate) thinking_visibility_error: Option<&'static str>,
    pub(crate) run_id: String,
    pub(crate) run_step: String,
    pub(crate) workflow_name: String,
    pub(crate) workflow_options: Vec<WorkflowOption>,
    pub(crate) workflow_empty: bool,
    pub(crate) draft_message: String,
    pub(crate) environment_preview: Vec<PreviewLine>,
    pub(crate) environment_preview_error: &'static str,
    pub(crate) preview_ready: bool,
    pub(crate) sandbox_status: SandboxStatus,
    pub(crate) sandbox_cursor: String,
    pub(crate) quick_ready: bool,
    pub(crate) review_href: String,
    pub(crate) quick_task_finished: bool,
    pub(crate) host_unchanged: &'static str,
}

pub(crate) struct PreviewLine {
    pub(crate) text: String,
}

pub(crate) struct WorkflowOption {
    pub(crate) token: String,
    pub(crate) label: String,
    pub(crate) policy: String,
    pub(crate) selected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SandboxStatusKind {
    Ready,
    Active,
    Failed,
    Unavailable,
    Invalid,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SandboxStatus {
    pub(crate) kind: SandboxStatusKind,
    pub(crate) message: &'static str,
    pub(crate) href: String,
    pub(crate) route_label: &'static str,
}

impl SandboxStatus {
    pub(crate) fn from_parts(
        record: Option<&EnvironmentRecord>,
        latest: Option<&PreparationRecord>,
        ready_availability: Option<SnapshotAvailability>,
    ) -> Self {
        if ready_availability == Some(SnapshotAvailability::Available) {
            return Self {
                kind: SandboxStatusKind::Ready,
                message: "Sandbox is ready",
                href: String::new(),
                route_label: "",
            };
        }
        if latest.is_some_and(|item| item.state.is_active()) {
            return Self {
                kind: SandboxStatusKind::Active,
                message: "Sandbox preparation is in progress",
                href: String::new(),
                route_label: "",
            };
        }
        let (href, route_label) = match record {
            Some(record) => (
                format!("/environments/{}/configuration", record.id.as_hex()),
                "Environment configuration",
            ),
            None => ("/environments".to_owned(), "Environments"),
        };
        if latest.is_some_and(|item| {
            matches!(
                item.state,
                PreparationState::Failed | PreparationState::Interrupted
            )
        }) {
            return Self {
                kind: SandboxStatusKind::Failed,
                message: "Sandbox preparation failed",
                href,
                route_label,
            };
        }
        if ready_availability == Some(SnapshotAvailability::Corrupt) {
            return Self {
                kind: SandboxStatusKind::Invalid,
                message: "Sandbox snapshot is invalid",
                href,
                route_label,
            };
        }
        Self {
            kind: SandboxStatusKind::Unavailable,
            message: "Sandbox is unavailable",
            href,
            route_label,
        }
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.kind == SandboxStatusKind::Ready
    }

    pub(crate) fn is_active(&self) -> bool {
        self.kind == SandboxStatusKind::Active
    }
}

impl ChatViewModel {
    pub(crate) fn from_session(
        record: &AgentRecord,
        session: &SessionSnapshot,
        vault: &ProviderVault,
        catalogue: &ModelCatalogue,
        models_dev: &ModelsDevCatalogue,
        error: &'static str,
        desk_error: &'static str,
    ) -> Self {
        Self::from_parts(
            record,
            &vault.desk_providers(),
            catalogue,
            models_dev,
            &session.turns,
            session.job.as_ref(),
            session.session_busy,
            error,
            desk_error,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        record: &AgentRecord,
        providers: &[DeskProvider],
        catalogue: &ModelCatalogue,
        models_dev: &ModelsDevCatalogue,
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
        let mut job_status = String::new();
        let mut run_id = String::new();
        let mut run_step = String::new();
        let mut workflow_name = String::new();
        if let Some(job) = job {
            run_id = job.run_id.as_hex();
            run_step = job.step_label.clone();
            workflow_name = job.workflow_name.clone();
            if !job.output.is_empty() && views.len() == job.assistant_index {
                views.push(assistant_reply_turn(
                    job.assistant_index,
                    &job.output,
                    job.status == JobStatus::Running,
                ));
            }
            if job.status == JobStatus::Running {
                job_id = job.id.as_hex();
                cursor = job.latest_seq;
                job_active = true;
                job_status = if job.cancel_requested {
                    "Stopping".to_owned()
                } else if job.step_label.is_empty() {
                    "Working".to_owned()
                } else {
                    job.step_label.clone()
                };
            }
        }
        let selected = providers.iter().find(|provider| provider.selected);
        let model = selected
            .map(|provider| provider.model.clone())
            .unwrap_or_default();
        let effective_thinking = selected.and_then(|provider| {
            models_dev.effective_effort(provider.kind, &provider.model, provider.thinking.as_ref())
        });
        let thinking_options = selected
            .map(|provider| {
                thinking_options(
                    models_dev,
                    provider.kind,
                    &provider.model,
                    effective_thinking.as_ref(),
                )
            })
            .unwrap_or_default();
        let (favourite_models, catalogue_models) = selected
            .map(|provider| {
                model_options(
                    provider.kind,
                    models_dev,
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
                    thinking: models_dev
                        .effective_effort(
                            provider.kind,
                            &provider.model,
                            provider.thinking.as_ref(),
                        )
                        .map(|value| value.as_str().to_owned())
                        .unwrap_or_default(),
                    efforts_json: efforts_json(models_dev, provider.kind, &provider.model),
                    selected: provider.selected,
                })
                .collect(),
            model,
            thinking_options,
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
            document_title: DOCUMENT_TITLE.to_owned(),
            project_id: String::new(),
            project_name: String::new(),
            project_path: String::new(),
            project_available: true,
            desk_href: String::new(),
            agent_id: record.id.as_hex(),
            agent_name: record.name.clone(),
            agent_choices: Vec::new(),
            show_thinking: false,
            thinking_visibility_error: None,
            run_id,
            run_step,
            workflow_name,
            workflow_options: Vec::new(),
            workflow_empty: false,
            draft_message: String::new(),
            environment_preview: Vec::new(),
            environment_preview_error: "",
            preview_ready: false,
            sandbox_status: SandboxStatus::from_parts(None, None, None),
            sandbox_cursor: String::new(),
            quick_ready: false,
            review_href: String::new(),
            quick_task_finished: false,
            host_unchanged: crate::workflows::HOST_UNCHANGED,
        }
    }

    pub(crate) fn with_project(
        mut self,
        project: &ProjectRecord,
        record: &AgentRecord,
        eligible: &[AgentRecord],
    ) -> Self {
        self.document_title = format!("{} | Power Plant", project.name);
        self.project_id = project.id.as_hex();
        self.project_name = project.name.clone();
        self.project_path = project.host_path.to_string_lossy().into_owned();
        self.project_available = project.host_path_is_available();
        self.desk_href = crate::projects::desk_path(&project.id, &record.id);
        self.agent_choices = eligible
            .iter()
            .map(|agent| AgentChoice {
                name: agent.name.clone(),
                href: crate::projects::desk_path(&project.id, &agent.id),
                selected: agent.id == record.id,
            })
            .collect();
        self
    }

    pub(crate) fn desk_settings(&self) -> DeskSettingsContents<'_> {
        DeskSettingsContents {
            providers: &self.providers,
            model: &self.model,
            thinking_options: &self.thinking_options,
            favourite_models: &self.favourite_models,
            catalogue_models: &self.catalogue_models,
            catalogue_pending: self.catalogue_pending,
            desk_error: self.desk_error,
            job_active: self.session_busy,
            project_id: &self.project_id,
            agent_id: &self.agent_id,
        }
    }

    pub(crate) fn desk_model_catalogue(&self) -> DeskModelCatalogueContents<'_> {
        DeskModelCatalogueContents {
            favourite_models: &self.favourite_models,
            catalogue_models: &self.catalogue_models,
            catalogue_pending: self.catalogue_pending,
        }
    }

    pub(crate) fn thinking_control(&self) -> ThinkingControlContents<'_> {
        ThinkingControlContents {
            thinking_options: &self.thinking_options,
            job_active: self.session_busy,
        }
    }

    pub(crate) fn sandbox_observe(&self) -> SandboxStatusContents<'_> {
        SandboxStatusContents {
            sandbox_status: &self.sandbox_status,
            desk_href: &self.desk_href,
            sandbox_cursor: &self.sandbox_cursor,
            workflow_options: &self.workflow_options,
        }
    }

    pub(crate) fn composer(&self) -> ComposerContents<'_> {
        ComposerContents {
            error: self.error,
            session_busy: self.session_busy,
            project_id: &self.project_id,
            project_available: self.project_available,
            desk_href: &self.desk_href,
            workflow_options: &self.workflow_options,
            workflow_empty: self.workflow_empty,
            draft_message: &self.draft_message,
            environment_preview: &self.environment_preview,
            environment_preview_error: self.environment_preview_error,
            preview_ready: self.preview_ready,
            quick_ready: self.quick_ready,
        }
    }

    pub(crate) fn job_observe(&self) -> JobObserveContents<'_> {
        self.job_observe_with("")
    }

    pub(crate) fn job_observe_with<'a>(&'a self, job_error: &'a str) -> JobObserveContents<'a> {
        JobObserveContents {
            job_error,
            job_id: &self.job_id,
            cursor: self.cursor,
            job_active: self.job_active,
            job_status: &self.job_status,
            desk_href: &self.desk_href,
            run_id: &self.run_id,
            run_step: &self.run_step,
            workflow_name: &self.workflow_name,
            review_href: &self.review_href,
            quick_task_finished: self.quick_task_finished,
            host_unchanged: self.host_unchanged,
        }
    }
}

pub(crate) fn user_turn(index: usize, text: &str) -> TurnView {
    TurnView {
        id: turn_id(index),
        is_user: true,
        response_html: format!("<p>{}</p>", markdown::escape_plain(text)),
        activities: Vec::new(),
        streaming: false,
        thinking_active: false,
    }
}

pub(crate) fn assistant_reply_turn(
    index: usize,
    reply: &AssistantReply,
    streaming: bool,
) -> TurnView {
    let mut activities = activity_views(reply);
    if streaming
        && reply.text.is_empty()
        && let Some(activity) = activities.last_mut()
        && activity.is_thinking
    {
        activity.thinking_active = true;
    }
    let thinking_active = streaming && reply.text.is_empty() && activities.is_empty();
    TurnView {
        id: turn_id(index),
        is_user: false,
        response_html: markdown::render(&reply.text),
        activities,
        streaming,
        thinking_active,
    }
}

fn activity_views(reply: &AssistantReply) -> Vec<ActivityView> {
    let mut views = Vec::new();
    if reply.activity.is_empty() {
        push_thinking_view(&mut views, &reply.thinking);
        for tool in &reply.tools {
            push_tool_view(&mut views, tool);
        }
        return views;
    }
    for activity in &reply.activity {
        match activity {
            AssistantActivity::Thinking(thinking) => push_thinking_view(&mut views, thinking),
            AssistantActivity::Tool(tool) => push_tool_view(&mut views, tool),
        }
    }
    views
}

fn push_thinking_view(views: &mut Vec<ActivityView>, thinking: &str) {
    if thinking.is_empty() {
        return;
    }
    views.push(ActivityView {
        is_thinking: true,
        thinking_html: markdown::render(thinking),
        tools: Vec::new(),
        thinking_active: false,
    });
}

fn push_tool_view(views: &mut Vec<ActivityView>, tool: &ToolOutput) {
    let tool = tool_output_view(tool);
    if let Some(activity) = views.last_mut()
        && !activity.is_thinking
    {
        activity.tools.push(tool);
        return;
    }
    views.push(ActivityView {
        is_thinking: false,
        thinking_html: String::new(),
        tools: vec![tool],
        thinking_active: false,
    });
}

pub(crate) fn turn_id(index: usize) -> String {
    format!("turn-{}", index + 1)
}

#[derive(Template)]
#[template(path = "chat/templates/thinking_visibility.html")]
pub(crate) struct ThinkingVisibilityControl {
    pub(crate) show_thinking: bool,
    pub(crate) thinking_visibility_error: Option<&'static str>,
}

#[derive(Template)]
#[template(path = "projects/templates/desk.html", block = "desk_status")]
pub(crate) struct DeskStatusContents<'a> {
    pub(crate) review_href: &'a str,
    pub(crate) job_active: bool,
    pub(crate) quick_task_finished: bool,
    pub(crate) job_status: &'a str,
}

impl<'a> DeskStatusContents<'a> {
    pub(crate) fn active(status: &'a str) -> Self {
        Self {
            review_href: "",
            job_active: true,
            quick_task_finished: false,
            job_status: status,
        }
    }

    pub(crate) fn idle(review_href: &'a str, quick_task_finished: bool) -> Self {
        Self {
            review_href,
            job_active: false,
            quick_task_finished,
            job_status: "",
        }
    }
}

#[derive(Template)]
#[template(path = "projects/templates/desk.html", block = "desk_settings")]
pub(crate) struct DeskSettingsContents<'a> {
    pub(crate) providers: &'a [DeskProviderOption],
    pub(crate) model: &'a str,
    pub(crate) thinking_options: &'a [ThinkingOption],
    pub(crate) favourite_models: &'a [ModelOption],
    pub(crate) catalogue_models: &'a [ModelOption],
    pub(crate) catalogue_pending: bool,
    pub(crate) desk_error: &'a str,
    pub(crate) job_active: bool,
    pub(crate) project_id: &'a str,
    pub(crate) agent_id: &'a str,
}

#[derive(Template)]
#[template(path = "projects/templates/desk.html", block = "desk_model_catalogue")]
pub(crate) struct DeskModelCatalogueContents<'a> {
    pub(crate) favourite_models: &'a [ModelOption],
    pub(crate) catalogue_models: &'a [ModelOption],
    pub(crate) catalogue_pending: bool,
}

#[derive(Template)]
#[template(path = "projects/templates/desk.html", block = "thinking_control")]
pub(crate) struct ThinkingControlContents<'a> {
    pub(crate) thinking_options: &'a [ThinkingOption],
    pub(crate) job_active: bool,
}

#[derive(Template)]
#[template(path = "projects/templates/desk.html", block = "transcript")]
pub(crate) struct TranscriptContents<'a> {
    pub(crate) turns: &'a [TurnView],
}

#[derive(Template)]
#[template(path = "projects/templates/desk.html", block = "composer")]
pub(crate) struct ComposerContents<'a> {
    pub(crate) error: &'a str,
    pub(crate) session_busy: bool,
    pub(crate) project_id: &'a str,
    pub(crate) project_available: bool,
    pub(crate) desk_href: &'a str,
    pub(crate) workflow_options: &'a [WorkflowOption],
    pub(crate) workflow_empty: bool,
    pub(crate) draft_message: &'a str,
    pub(crate) environment_preview: &'a [PreviewLine],
    pub(crate) environment_preview_error: &'static str,
    pub(crate) preview_ready: bool,
    pub(crate) quick_ready: bool,
}

#[derive(Template)]
#[template(path = "projects/templates/desk.html", block = "sandbox_status")]
pub(crate) struct SandboxStatusContents<'a> {
    pub(crate) sandbox_status: &'a SandboxStatus,
    pub(crate) desk_href: &'a str,
    pub(crate) sandbox_cursor: &'a str,
    pub(crate) workflow_options: &'a [WorkflowOption],
}

#[derive(Template)]
#[template(path = "projects/templates/desk.html", block = "job_observe")]
pub(crate) struct JobObserveContents<'a> {
    pub(crate) job_error: &'a str,
    pub(crate) job_id: &'a str,
    pub(crate) cursor: u64,
    pub(crate) job_active: bool,
    pub(crate) job_status: &'a str,
    pub(crate) desk_href: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) run_step: &'a str,
    pub(crate) workflow_name: &'a str,
    pub(crate) review_href: &'a str,
    pub(crate) quick_task_finished: bool,
    pub(crate) host_unchanged: &'static str,
}

impl<'a> JobObserveContents<'a> {
    pub(crate) fn idle(
        error: &'a str,
        desk_href: &'a str,
        run_id: &'a str,
        run_step: &'a str,
        workflow_name: &'a str,
        review_href: &'a str,
        quick_task_finished: bool,
    ) -> Self {
        Self {
            job_error: error,
            job_id: "",
            cursor: 0,
            job_active: false,
            job_status: "",
            desk_href,
            run_id,
            run_step,
            workflow_name,
            review_href,
            quick_task_finished,
            host_unchanged: crate::workflows::HOST_UNCHANGED,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observing(
        job_id: &'a str,
        cursor: u64,
        status: &'a str,
        error: &'a str,
        desk_href: &'a str,
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
            desk_href,
            run_id,
            run_step,
            workflow_name,
            review_href: "",
            quick_task_finished: false,
            host_unchanged: crate::workflows::HOST_UNCHANGED,
        }
    }
}

#[derive(Template)]
#[template(path = "chat/templates/job_cursor.html")]
pub(crate) struct JobCursorContents<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) cursor: u64,
}

#[derive(Template)]
#[template(path = "chat/templates/turn_article.html")]
pub(crate) struct TurnArticle<'a> {
    pub(crate) turn: &'a TurnView,
}

#[derive(Template)]
#[template(path = "chat/templates/turn_body.html")]
pub(crate) struct TurnBody<'a> {
    pub(crate) turn: &'a TurnView,
}

fn thinking_options(
    catalogue: &ModelsDevCatalogue,
    kind: ProviderKind,
    model: &str,
    current: Option<&ThinkingEffort>,
) -> Vec<ThinkingOption> {
    catalogue
        .efforts(kind, model)
        .into_iter()
        .map(|effort| ThinkingOption {
            value: effort.as_str().to_owned(),
            label: effort.label(),
            selected: current == Some(&effort),
        })
        .collect()
}

fn efforts_json(catalogue: &ModelsDevCatalogue, kind: ProviderKind, model: &str) -> String {
    let values: Vec<_> = catalogue
        .efforts(kind, model)
        .into_iter()
        .map(|value| value.as_str().to_owned())
        .collect();
    serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_owned())
}

fn model_options(
    kind: ProviderKind,
    metadata: &ModelsDevCatalogue,
    current: &str,
    favourites: &[String],
    listed: &[String],
) -> (Vec<ModelOption>, Vec<ModelOption>) {
    let current_favourite = favourites.iter().any(|item| item == current);
    let favourite_models = favourites
        .iter()
        .map(|id| ModelOption {
            id: id.clone(),
            efforts_json: efforts_json(metadata, kind, id),
            selected: id == current,
        })
        .collect();
    let mut catalogue_models: Vec<ModelOption> = listed
        .iter()
        .filter(|id| !favourites.contains(id))
        .map(|id| ModelOption {
            id: id.clone(),
            efforts_json: efforts_json(metadata, kind, id),
            selected: id == current,
        })
        .collect();
    if !current.is_empty() && !current_favourite && !listed.iter().any(|id| id == current) {
        catalogue_models.insert(
            0,
            ModelOption {
                id: current.to_owned(),
                efforts_json: efforts_json(metadata, kind, current),
                selected: true,
            },
        );
    }
    (favourite_models, catalogue_models)
}

fn turn_view(index: usize, turn: &ChatTurn) -> TurnView {
    match turn.role {
        Role::User => user_turn(index, &turn.text),
        Role::Assistant => assistant_reply_turn(
            index,
            &AssistantReply {
                text: turn.text.clone(),
                thinking: turn.thinking.clone(),
                tools: turn.tools.clone(),
                activity: turn.activity.clone(),
            },
            false,
        ),
    }
}

fn tool_output_view(tool: &ToolOutput) -> ToolOutputView {
    ToolOutputView {
        label: tool.label.clone(),
        output: tool.output.clone(),
        preview: tool_preview(&tool.output),
    }
}

fn tool_preview(output: &str) -> String {
    const MAXIMUM_PREVIEW_BYTES: usize = 720;
    const MAXIMUM_PREVIEW_LINES: usize = 6;
    let line_end = output
        .char_indices()
        .filter(|(_, character)| *character == '\n')
        .nth(MAXIMUM_PREVIEW_LINES - 1)
        .map(|(index, _)| index)
        .unwrap_or(output.len());
    let mut end = line_end.min(MAXIMUM_PREVIEW_BYTES).min(output.len());
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = end < output.len();
    let mut preview = output[..end].trim_end().to_owned();
    if truncated {
        preview.push_str("\n…");
    }
    preview
}
