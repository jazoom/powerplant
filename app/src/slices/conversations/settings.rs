use axum::{
    Form,
    extract::{Path, State},
    response::Response,
};
use hypergraft::{PatchGraft, PatchStatus};
use serde::Deserialize;

use crate::{
    agents::ToolId,
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
    ExecutionSettings::new(
        selection,
        form.instructions.clone(),
        parse_tools(&form.tool_values())?,
    )
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
                .with_settings_fields(
                    &state,
                    &form.provider,
                    &form.model,
                    &form.thinking,
                    form.instructions.clone(),
                    &form.tool_values(),
                ),
        );
    };
    let settings = match validate(&state, &form) {
        Ok(settings) => settings,
        Err(error) => {
            return render_detail_command(
                graft,
                PatchStatus::UnprocessableEntity,
                detail_view(&state, session.0, &record, &record.title, error).with_settings_fields(
                    &state,
                    &form.provider,
                    &form.model,
                    &form.thinking,
                    form.instructions.clone(),
                    &form.tool_values(),
                ),
            );
        }
    };
    let selection = settings.model.clone();
    match state
        .conversations
        .update_execution_settings(&record.id, revision, settings)
    {
        Ok(updated) => {
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
                .with_settings_fields(
                    &state,
                    &form.provider,
                    &form.model,
                    &form.thinking,
                    form.instructions.clone(),
                    &form.tool_values(),
                ),
        ),
    }
}

#[cfg(test)]
mod tests;
