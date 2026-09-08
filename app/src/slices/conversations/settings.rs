use axum::{
    Form,
    extract::{Path, State},
    response::Response,
};
use hypergraft::{GraftRequest, PatchGraft, PatchStatus};
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
    pub(super) location: String,
    pub(super) host_approval: String,
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
            location: &self.location,
            host_approval: &self.host_approval,
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
    let location = crate::execution::ToolLocation::parse(if form.location.trim().is_empty() {
        "sandbox"
    } else {
        form.location.trim()
    })
    .ok_or("Choose where tools run.")?;
    let host_approval =
        crate::execution::HostApprovalPolicy::parse(if form.host_approval.trim().is_empty() {
            "ask-each-time"
        } else {
            form.host_approval.trim()
        })
        .ok_or("Choose host command approval.")?;
    ExecutionSettings::new(
        selection,
        form.instructions.clone(),
        parse_tools(&form.tool_values())?,
        environment,
    )
    .and_then(|settings| settings.with_network(network))
    .map(|settings| {
        settings
            .with_location(location)
            .with_host_approval(host_approval)
    })
    .ok_or("Enter instructions within 32 KiB without unsupported control characters.")
}

#[derive(Deserialize)]
pub(super) struct PresetChoiceForm {
    pub(super) revision: String,
    pub(super) preset: String,
}

#[derive(Deserialize)]
pub(super) struct PresetApplyForm {
    pub(super) revision: String,
    pub(super) preset_preview: String,
}

#[derive(Deserialize)]
pub(super) struct PresetSaveForm {
    pub(super) revision: String,
    pub(super) name: String,
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
            || model.settings.location != settings.location
            || model.settings.host_approval != settings.host_approval
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

pub(super) async fn save_preset(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<PresetSaveForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return preset_saved_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    };
    if revision != record.revision {
        return preset_saved_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    }
    let Some(configuration) = &record.model else {
        return preset_saved_error(
            &state,
            session.0,
            graft,
            &record,
            "Choose conversation settings first.",
        );
    };
    let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
        return preset_saved_error(
            &state,
            session.0,
            graft,
            &record,
            crate::local_data::HOST_PATH_RESET_PENDING,
        );
    };
    let name = if form.name.trim().is_empty() {
        crate::presets::suggested_name(&configuration.settings)
    } else {
        form.name.clone()
    };
    match state.presets.create(
        &name,
        configuration.settings.clone(),
        crate::presets::PresetProvenance::Conversation {
            id: record.id,
            title: record.title.clone(),
        },
    ) {
        Ok(_) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &record, &record.title, "").open_settings(),
        ),
        Err(error) => preset_saved_error(&state, session.0, graft, &record, error.message()),
    }
}

pub(super) async fn preview_preset(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<PresetChoiceForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return preset_saved_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    };
    if revision != record.revision {
        return preset_saved_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    }
    if record.active_job.is_some() || super::has_pending_review(&state, record.id) {
        return preset_saved_error(
            &state,
            session.0,
            graft,
            &record,
            "Finish or discard the current work before you apply a preset.",
        );
    }
    let preview = crate::presets::PresetId::parse(&form.preset)
        .ok_or(crate::presets::PresetError::Missing)
        .and_then(|id| {
            state.presets.preview(
                session.0,
                id,
                crate::presets::PresetDestination::Conversation(record.id, revision),
            )
        });
    match preview {
        Ok(preview) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &record, &record.title, "")
                .with_preset_preview(preset_preview_view(&state, preview)),
        ),
        Err(error) => preset_saved_error(&state, session.0, graft, &record, error.message()),
    }
}

pub(super) async fn apply_preset(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<PresetApplyForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return preset_saved_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    };
    if revision != record.revision
        || record.active_job.is_some()
        || super::has_pending_review(&state, record.id)
    {
        return preset_saved_error(&state, session.0, graft, &record, REVISION_MESSAGE);
    }
    let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
        return preset_saved_error(
            &state,
            session.0,
            graft,
            &record,
            crate::local_data::HOST_PATH_RESET_PENDING,
        );
    };
    let preset = match state.presets.consume_preview(
        session.0,
        &form.preset_preview,
        &crate::presets::PresetDestination::Conversation(record.id, revision),
    ) {
        Ok(preset) => preset,
        Err(error) => {
            return preset_saved_error(&state, session.0, graft, &record, error.message());
        }
    };
    match state
        .conversations
        .apply_preset(&record.id, revision, &preset)
    {
        Ok(updated) => {
            state.access_consent.invalidate_conversation(record.id);
            render_detail_command(
                graft,
                PatchStatus::Ok,
                detail_view(&state, session.0, &updated, &updated.title, "").open_settings(),
            )
        }
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("apply conversation preset", error))
        }
        Err(error) => preset_saved_error(&state, session.0, graft, &record, error.message()),
    }
}

pub(super) async fn save_draft_preset(
    State(state): State<AppState>,
    session: RequiredSession,
    _graft: PatchGraft,
    Form(form): Form<super::new::NewForm>,
) -> AppResult<Response> {
    let result = super::new::settings_snapshot(&state, session.0, &form)
        .and_then(|model| model.ok_or("Choose conversation settings first."));
    let status;
    let error;
    if let Ok(configuration) = result {
        let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
            return render_draft_preset(
                &state,
                session.0,
                form,
                PatchStatus::Conflict,
                crate::local_data::HOST_PATH_RESET_PENDING,
            );
        };
        let name = if form.preset_name.trim().is_empty() {
            crate::presets::suggested_name(&configuration.settings)
        } else {
            form.preset_name.clone()
        };
        match state.presets.create(
            &name,
            configuration.settings,
            crate::presets::PresetProvenance::Draft,
        ) {
            Ok(_) => {
                status = PatchStatus::Ok;
                error = "";
            }
            Err(store_error) => {
                status = PatchStatus::UnprocessableEntity;
                error = store_error.message();
            }
        }
    } else {
        status = PatchStatus::UnprocessableEntity;
        error = result
            .err()
            .unwrap_or("Choose conversation settings first.");
    }
    render_draft_preset(&state, session.0, form, status, error)
}

pub(super) async fn preview_draft_preset(
    State(state): State<AppState>,
    session: RequiredSession,
    _graft: PatchGraft,
    Form(form): Form<super::new::NewForm>,
) -> AppResult<Response> {
    let preview = crate::presets::PresetId::parse(&form.preset)
        .ok_or(crate::presets::PresetError::Missing)
        .and_then(|id| {
            state.presets.preview(
                session.0,
                id,
                crate::presets::PresetDestination::Draft(form.consent_nonce()),
            )
        });
    match preview {
        Ok(preview) => {
            let view = super::page::ConversationDetailView::from_new(&state, session.0, form, "")
                .with_preset_preview(preset_preview_view(&state, preview));
            super::render_detail(
                &state,
                session.0,
                GraftRequest::Patch,
                PatchStatus::Ok,
                view,
            )
        }
        Err(error) => render_draft_preset(
            &state,
            session.0,
            form,
            PatchStatus::UnprocessableEntity,
            error.message(),
        ),
    }
}

pub(super) async fn apply_draft_preset(
    State(state): State<AppState>,
    session: RequiredSession,
    _graft: PatchGraft,
    Form(mut form): Form<super::new::NewForm>,
) -> AppResult<Response> {
    let preset = match state.presets.apply_draft_preview(
        session.0,
        &form.preset_preview,
        form.consent_nonce(),
    ) {
        Ok(preset) => preset,
        Err(error) => {
            return render_draft_preset(
                &state,
                session.0,
                form,
                PatchStatus::UnprocessableEntity,
                error.message(),
            );
        }
    };
    apply_settings_to_draft(&mut form, &preset);
    form.consent_reference.clear();
    form.consent_request.clear();
    form.pending_directory.clear();
    render_draft_preset(&state, session.0, form, PatchStatus::Ok, "")
}

fn apply_settings_to_draft(form: &mut super::new::NewForm, preset: &crate::presets::PresetRecord) {
    form.preset = preset.id.as_hex();
    copy_settings_to_draft(form, &preset.settings);
}

pub(super) fn copy_settings_to_draft(
    form: &mut super::new::NewForm,
    settings: &crate::execution::ExecutionSettings,
) {
    form.provider = settings.model.provider.as_str().to_owned();
    form.model = settings.model.model.clone();
    form.thinking = settings
        .model
        .thinking
        .as_ref()
        .map(|effort| effort.as_str().to_owned())
        .unwrap_or_default();
    form.instructions = settings.instructions.clone();
    form.tool_list = if settings.tools.contains(&ToolId::List) {
        "list".to_owned()
    } else {
        String::new()
    };
    form.tool_read = if settings.tools.contains(&ToolId::Read) {
        "read".to_owned()
    } else {
        String::new()
    };
    form.tool_write = if settings.tools.contains(&ToolId::Write) {
        "write".to_owned()
    } else {
        String::new()
    };
    form.tool_run = if settings.tools.contains(&ToolId::Run) {
        "run".to_owned()
    } else {
        String::new()
    };
    form.location = settings.location.as_str().to_owned();
    form.host_approval = settings.host_approval.as_str().to_owned();
    form.environment = settings.environment.as_hex();
    form.network = settings.network.as_str().to_owned();
    form.network_domains = settings.network.domains().join("\n");
    form.set_directories(&settings.directories);
}

fn preset_preview_view(
    state: &AppState,
    preview: crate::presets::PresetPreview,
) -> super::page::PresetPreviewView {
    let settings = &preview.record.settings;
    super::page::PresetPreviewView {
        token: preview.token,
        name: preview.record.name,
        model: format!(
            "{} · {}",
            settings.model.provider.label(),
            settings.model.model
        ),
        thinking: settings
            .model
            .thinking
            .as_ref()
            .map(|effort| effort.as_str().to_owned())
            .unwrap_or_else(|| "Default".to_owned()),
        instructions: settings.instructions.clone(),
        environment: state
            .environments
            .get(&settings.environment)
            .map(|record| record.name)
            .unwrap_or_else(|| "Unavailable environment".to_owned()),
        tools: if settings.tools.is_empty() {
            "No tools".to_owned()
        } else {
            settings
                .tools
                .iter()
                .map(|tool| tool.label())
                .collect::<Vec<_>>()
                .join(", ")
        },
        network: match &settings.network {
            NetworkAccess::None => "Off".to_owned(),
            NetworkAccess::Public => "Public internet".to_owned(),
            NetworkAccess::Restricted(domains) => format!("Restricted: {}", domains.join(", ")),
        },
        location: match settings.location {
            crate::execution::ToolLocation::Sandbox => "Sandbox".to_owned(),
            crate::execution::ToolLocation::Host => {
                if settings.host_approval.automatic() {
                    "This computer · Run without approval".to_owned()
                } else {
                    "This computer · Ask each time".to_owned()
                }
            }
        },
        directories: settings
            .directories
            .iter()
            .map(|grant| {
                format!(
                    "{} · {} · {}",
                    grant.host_path.display(),
                    crate::slices::execution_settings::page::directory_access_label(grant.access),
                    if grant.is_available() {
                        "Available"
                    } else {
                        "Unavailable"
                    }
                )
            })
            .collect(),
    }
}

fn render_draft_preset(
    state: &AppState,
    session: crate::sessions::SessionId,
    form: super::new::NewForm,
    status: PatchStatus,
    error: &'static str,
) -> AppResult<Response> {
    let view =
        super::page::ConversationDetailView::from_new(state, session, form, error).open_settings();
    super::render_detail(state, session, GraftRequest::Patch, status, view)
}

fn preset_saved_error(
    state: &AppState,
    session: crate::sessions::SessionId,
    graft: PatchGraft,
    record: &crate::conversations::ConversationRecord,
    error: &'static str,
) -> AppResult<Response> {
    render_detail_command(
        graft,
        if error == REVISION_MESSAGE || error == crate::local_data::HOST_PATH_RESET_PENDING {
            PatchStatus::Conflict
        } else {
            PatchStatus::UnprocessableEntity
        },
        detail_view(state, session, record, &record.title, error).open_settings(),
    )
}

#[derive(Deserialize)]
pub(super) struct HostConsentForm {
    pub(super) revision: String,
    #[serde(default)]
    pub(super) consent_request: String,
}

pub(super) async fn request_host(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(_form): Form<HostConsentForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(model) = record.model.as_ref() else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose conversation settings first.",
            )
            .open_settings(),
        );
    };
    if model.settings.location != crate::execution::ToolLocation::Host {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose This computer before you approve unrestricted host access.",
            )
            .open_settings(),
        );
    }
    let request =
        match state
            .access_consent
            .request_host_conversation(session.0, record.id, &model.settings)
        {
            Ok(request) => request,
            Err(_) => {
                return render_detail_command(
                    graft,
                    PatchStatus::UnprocessableEntity,
                    detail_view(
                        &state,
                        session.0,
                        &record,
                        &record.title,
                        "Power Plant could not start host access approval. Try again.",
                    )
                    .open_settings(),
                );
            }
        };
    render_detail_command(
        graft,
        PatchStatus::Ok,
        detail_view(&state, session.0, &record, &record.title, "")
            .open_settings()
            .with_host_consent_request(request),
    )
}

pub(super) async fn approve_host(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<HostConsentForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE)
                .open_settings(),
        );
    };
    if revision != record.revision {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE)
                .open_settings(),
        );
    }
    let Some(model) = record.model.as_ref() else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose conversation settings first.",
            )
            .open_settings(),
        );
    };
    if state
        .access_consent
        .approve_host_conversation(&form.consent_request, session.0, record.id, &model.settings)
        .is_err()
    {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "The host access request expired or changed. Review it again.",
            )
            .open_settings(),
        );
    }
    render_detail_command(
        graft,
        PatchStatus::Ok,
        detail_view(&state, session.0, &record, &record.title, "").open_settings(),
    )
}

pub(super) async fn request_host_draft(
    State(state): State<AppState>,
    session: RequiredSession,
    _graft: PatchGraft,
    Form(form): Form<super::new::NewForm>,
) -> AppResult<Response> {
    let settings = match super::new::settings_snapshot(&state, session.0, &form) {
        Ok(Some(model)) => model.settings,
        Ok(None) => {
            return render_draft_preset(
                &state,
                session.0,
                form,
                PatchStatus::UnprocessableEntity,
                "Choose a stored provider.",
            );
        }
        Err(error) => {
            return render_draft_preset(
                &state,
                session.0,
                form,
                PatchStatus::UnprocessableEntity,
                error,
            );
        }
    };
    if settings.location != crate::execution::ToolLocation::Host {
        return render_draft_preset(
            &state,
            session.0,
            form,
            PatchStatus::UnprocessableEntity,
            "Choose This computer before you approve unrestricted host access.",
        );
    }
    let mut form = form;
    match state
        .access_consent
        .request_host_draft(session.0, &form.consent_nonce(), &settings)
    {
        Ok(request) => form.host_consent_request = request,
        Err(_) => {
            return render_draft_preset(
                &state,
                session.0,
                form,
                PatchStatus::UnprocessableEntity,
                "Power Plant could not start host access approval. Try again.",
            );
        }
    }
    render_draft_preset(&state, session.0, form, PatchStatus::Ok, "")
}

pub(super) async fn approve_host_draft(
    State(state): State<AppState>,
    session: RequiredSession,
    _graft: PatchGraft,
    Form(mut form): Form<super::new::NewForm>,
) -> AppResult<Response> {
    let settings = match super::new::settings_snapshot(&state, session.0, &form) {
        Ok(Some(model)) => model.settings,
        Ok(None) => {
            return render_draft_preset(
                &state,
                session.0,
                form,
                PatchStatus::UnprocessableEntity,
                "Choose a stored provider.",
            );
        }
        Err(error) => {
            return render_draft_preset(
                &state,
                session.0,
                form,
                PatchStatus::UnprocessableEntity,
                error,
            );
        }
    };
    let reference = match state.access_consent.approve_host_draft(
        &form.host_consent_request,
        session.0,
        &form.consent_nonce(),
        &settings,
    ) {
        Ok(reference) => reference,
        Err(_) => {
            return render_draft_preset(
                &state,
                session.0,
                form,
                PatchStatus::UnprocessableEntity,
                "The host access request expired or changed. Review it again.",
            );
        }
    };
    if !form.consent_reference.is_empty() {
        form.consent_reference.push(',');
    }
    form.consent_reference.push_str(&reference);
    form.host_consent_request.clear();
    render_draft_preset(&state, session.0, form, PatchStatus::Ok, "")
}

#[cfg(test)]
mod tests;
