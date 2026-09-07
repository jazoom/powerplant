use axum::{
    Form,
    extract::{Path, State},
    response::Response,
};
use hypergraft::{PatchGraft, PatchStatus};
use serde::Deserialize;

use crate::{
    agents::{NetworkAccess, ToolId},
    conversations::ConversationError,
    error::{AppError, AppResult},
    execution::ExecutionSettings,
    providers::{ModelSelection, ProviderKind, ThinkingEffort},
    responses,
    sessions::RequiredSession,
    state::AppState,
};

use super::{
    REVISION_MESSAGE, detail_view, load_conversation, parse_revision, remember_selection,
    render_detail_command, status_for, valid_selection,
};

const CANCELLATION_WAIT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(250)
} else {
    std::time::Duration::from_secs(30)
};

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct SettingsForm {
    pub(super) revision: String,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) thinking: String,
    pub(super) instructions: String,
    pub(super) tool_list: String,
    pub(super) tool_read: String,
    pub(super) tool_write: String,
    pub(super) tool_run: String,
    pub(super) environment: String,
    pub(super) network: String,
    pub(super) network_domains: String,
}

#[derive(Deserialize)]
pub(super) struct EnvironmentSwitchForm {
    pub(super) revision: String,
    pub(super) environment: String,
    #[serde(default)]
    pub(super) job: String,
}

pub(super) fn parse_tools(values: &[String]) -> Result<Vec<ToolId>, &'static str> {
    let mut tools = Vec::with_capacity(values.len());
    for value in values {
        let tool = ToolId::parse(value.trim()).ok_or("Choose only available tools.")?;
        if tools.contains(&tool) {
            return Err("Choose each tool once.");
        }
        tools.push(tool);
    }
    Ok(tools)
}

impl SettingsForm {
    fn tool_values(&self) -> Vec<String> {
        [
            &self.tool_list,
            &self.tool_read,
            &self.tool_write,
            &self.tool_run,
        ]
        .into_iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect()
    }

    fn submitted_fields(&self) -> super::page::SubmittedSettingsFields<'_> {
        super::page::SubmittedSettingsFields {
            provider: &self.provider,
            model: &self.model,
            thinking: &self.thinking,
            instructions: self.instructions.clone(),
            tools: self.tool_values(),
            environment: &self.environment,
            network: &self.network,
            network_domains: &self.network_domains,
        }
    }
}

fn validate(state: &AppState, form: &SettingsForm) -> Result<ExecutionSettings, &'static str> {
    let provider = ProviderKind::parse(form.provider.trim()).ok_or("Choose a stored provider.")?;
    let has_thinking = !form.thinking.trim().is_empty();
    let thinking = has_thinking
        .then(|| ThinkingEffort::new(form.thinking.clone()))
        .flatten();
    if has_thinking && thinking.is_none() {
        return Err("Choose an available thinking effort.");
    }
    let selection = ModelSelection::new(provider, form.model.clone(), thinking)
        .ok_or("Enter a valid model name.")?;
    valid_selection(state, &selection)?;
    let environment = super::selected_environment(state, &form.environment)?;
    let network_mode = if form.network.trim().is_empty() {
        "none"
    } else {
        form.network.as_str()
    };
    let network = NetworkAccess::parse_form(network_mode, &form.network_domains)
        .map_err(|_| "Choose valid network access. Restricted access needs 1 to 32 domains.")?;
    ExecutionSettings::new(
        selection,
        form.instructions.clone(),
        parse_tools(&form.tool_values())?,
        environment,
    )
    .and_then(|settings| settings.with_network(network))
    .ok_or("Enter instructions within 32 KiB without unsupported control characters.")
}

pub(super) async fn update(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<SettingsForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE)
                .with_settings_fields(&state, form.submitted_fields()),
        );
    };
    if revision != record.revision {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE)
                .with_settings_fields(&state, form.submitted_fields()),
        );
    }
    let settings = match validate(&state, &form).and_then(|settings| {
        let directories = record
            .model
            .as_ref()
            .map(|model| model.settings.directories.clone())
            .unwrap_or_default();
        settings
            .with_directories(directories)
            .ok_or("The saved directory grants are not valid.")
    }) {
        Ok(settings) => settings,
        Err(error) => {
            return render_detail_command(
                graft,
                PatchStatus::UnprocessableEntity,
                detail_view(&state, session.0, &record, &record.title, error)
                    .with_settings_fields(&state, form.submitted_fields()),
            );
        }
    };
    let environment_changed = record
        .model
        .as_ref()
        .is_some_and(|model| model.settings.environment != settings.environment);
    if record.active_job.is_some() || super::has_pending_review(&state, record.id) {
        if environment_changed {
            if let Err(error) =
                replacement_environment_ready(&state, &record, settings.environment).await
            {
                return render_detail_command(
                    graft,
                    PatchStatus::UnprocessableEntity,
                    detail_view(&state, session.0, &record, &record.title, error)
                        .with_settings_fields(&state, form.submitted_fields()),
                );
            }
            return render_switch_preview(
                &state,
                session.0,
                graft,
                &record,
                settings.environment,
                Some(form.submitted_fields()),
            );
        }
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Finish or discard the current work before you change execution settings.",
            )
            .with_settings_fields(&state, form.submitted_fields()),
        );
    }
    let access_changed = record.model.as_ref().is_some_and(|model| {
        model.settings.tools != settings.tools
            || model.settings.network != settings.network
            || model.settings.environment != settings.environment
    });
    let selection = settings.model.clone();
    match state
        .conversations
        .update_execution_settings(&record.id, revision, settings)
    {
        Ok(updated) => {
            if access_changed {
                state.access_consent.invalidate_conversation(record.id);
            }
            let warning = remember_selection(&state, selection).err().unwrap_or("");
            render_detail_command(
                graft,
                PatchStatus::Ok,
                detail_view(&state, session.0, &updated, &updated.title, warning).open_settings(),
            )
        }
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store conversation settings", error))
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message())
                .with_settings_fields(&state, form.submitted_fields()),
        ),
    }
}

pub(super) async fn preview_environment_switch(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<EnvironmentSwitchForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return switch_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    };
    if revision != record.revision {
        return switch_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    }
    let environment = match super::selected_environment(&state, &form.environment) {
        Ok(environment) => environment,
        Err(error) => return switch_error(&state, session.0, graft, &record, error),
    };
    if record
        .model
        .as_ref()
        .is_some_and(|model| model.settings.environment == environment)
    {
        return switch_error(
            &state,
            session.0,
            graft,
            &record,
            "Choose a different environment.",
        );
    }
    if record.active_job.is_some() || super::has_pending_review(&state, record.id) {
        if let Err(error) = replacement_environment_ready(&state, &record, environment).await {
            return switch_error(&state, session.0, graft, &record, error);
        }
        return render_switch_preview(&state, session.0, graft, &record, environment, None);
    }
    save_environment(&state, session.0, graft, &record, revision, environment)
}

pub(super) async fn stop_and_switch_environment(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<EnvironmentSwitchForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return switch_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    };
    let Some(environment) = crate::environments::EnvironmentId::parse(&form.environment) else {
        return switch_error(
            &state,
            session.0,
            graft,
            &record,
            "Choose an available environment.",
        );
    };
    let Some(job_id) = crate::sessions::JobId::parse(&form.job) else {
        return switch_error(
            &state,
            session.0,
            graft,
            &record,
            "That task is no longer active.",
        );
    };
    if revision != record.revision || record.active_job != Some(job_id) {
        return switch_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    }
    if let Err(error) = replacement_environment_ready(&state, &record, environment).await {
        return switch_error(&state, session.0, graft, &record, error);
    }
    let Some(job) = state.sessions.conversation_job(record.id, job_id) else {
        return switch_error(
            &state,
            session.0,
            graft,
            &record,
            "That task is no longer active.",
        );
    };
    if !state
        .conversations
        .get(&record.id)
        .is_some_and(|current| current.revision == revision && current.active_job == Some(job_id))
    {
        return switch_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    }
    job.request_cancel();
    if !job.wait_for_terminal(CANCELLATION_WAIT).await {
        return switch_error(
            &state,
            session.0,
            graft,
            &record,
            "Power Plant is still stopping the task. The environment did not change.",
        );
    }
    let current = state
        .conversations
        .get(&record.id)
        .unwrap_or_else(|| record.clone());
    if current.model != record.model {
        return switch_error(&state, session.0, graft, &current, REVISION_MESSAGE);
    }
    if current.active_job.is_some() {
        return switch_error(
            &state,
            session.0,
            graft,
            &current,
            "Power Plant could not clean up the task. The environment did not change.",
        );
    }
    save_environment(
        &state,
        session.0,
        graft,
        &current,
        current.revision,
        environment,
    )
}

pub(crate) async fn replacement_environment_ready(
    state: &AppState,
    record: &crate::conversations::ConversationRecord,
    environment: crate::environments::EnvironmentId,
) -> Result<(), &'static str> {
    record.model.as_ref().ok_or("Choose a model first.")?;
    crate::workflows::validate_replacement_environment(
        &state.environments,
        &state.environment_snapshots,
        environment,
    )
    .await
    .map_err(|error| error.message())
}

fn render_switch_preview(
    state: &AppState,
    session: crate::sessions::SessionId,
    graft: PatchGraft,
    record: &crate::conversations::ConversationRecord,
    environment: crate::environments::EnvironmentId,
    fields: Option<super::page::SubmittedSettingsFields<'_>>,
) -> AppResult<Response> {
    let mut view = detail_view(state, session, record, &record.title, "");
    if let Some(fields) = fields {
        let effective_environment = view.environment_summary.clone();
        view = view.with_settings_fields(state, fields);
        view.environment_summary = effective_environment;
    }
    let gate = view.saved().and_then(|saved| saved.pending_gate.clone());
    render_detail_command(
        graft,
        PatchStatus::Ok,
        view.with_environment_switch(state, environment, gate.as_ref()),
    )
}

fn switch_error(
    state: &AppState,
    session: crate::sessions::SessionId,
    graft: PatchGraft,
    record: &crate::conversations::ConversationRecord,
    error: &'static str,
) -> AppResult<Response> {
    render_detail_command(
        graft,
        PatchStatus::Conflict,
        detail_view(state, session, record, &record.title, error).open_settings(),
    )
}

fn save_environment(
    state: &AppState,
    session: crate::sessions::SessionId,
    graft: PatchGraft,
    record: &crate::conversations::ConversationRecord,
    revision: u32,
    environment: crate::environments::EnvironmentId,
) -> AppResult<Response> {
    match state
        .conversations
        .select_environment(&record.id, revision, environment)
    {
        Ok(updated) => {
            state.access_consent.invalidate_conversation(record.id);
            render_detail_command(
                graft,
                PatchStatus::Ok,
                detail_view(state, session, &updated, &updated.title, "").open_settings(),
            )
        }
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store conversation environment", error))
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(state, session, record, &record.title, error.message()).open_settings(),
        ),
    }
}

#[cfg(test)]
mod tests;
