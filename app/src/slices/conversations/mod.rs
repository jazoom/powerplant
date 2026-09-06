mod job;
mod page;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::Response,
    routing::{get, post},
};
use hypergraft::{GraftRequest, PatchGraft, PatchStatus};
use serde::Deserialize;

use crate::{
    agents::AgentId,
    conversations::{
        ConversationError, ConversationId, ConversationModelConfiguration, ConversationRecord,
    },
    error::{AppError, AppResult},
    projects::ProjectId,
    providers::{ModelSelection, ProviderKind, ThinkingEffort},
    responses,
    sessions::{JobId, RequiredSession},
    state::AppState,
};

use self::page::{CatalogueView, ConversationDetailView, ConversationFormView, ModelSources};

const REVISION_MESSAGE: &str = "Reload the conversation and try again.";

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/conversations", get(catalogue).post(create))
        .route("/conversations/new", get(new_conversation))
        .route("/conversations/{conversation_id}", get(detail))
        .route(
            "/conversations/{conversation_id}/messages",
            post(send_message),
        )
        .route(
            "/conversations/{conversation_id}/cancel",
            post(cancel_message),
        )
        .route("/conversations/{conversation_id}/model", post(select_model))
        .route(
            "/conversations/{conversation_id}/preset",
            post(apply_preset),
        )
        .route(
            "/conversations/{conversation_id}/projects",
            post(attach_project),
        )
        .route(
            "/conversations/{conversation_id}/projects/{project_id}",
            post(detach_project),
        )
        .route(
            "/conversations/{conversation_id}/rename",
            post(rename_conversation),
        )
        .route(
            "/conversations/{conversation_id}/delete",
            post(delete_conversation),
        )
}

#[derive(Deserialize)]
struct ConversationForm {
    title: String,
}

#[derive(Deserialize)]
struct RenameForm {
    title: String,
    revision: String,
}

#[derive(Deserialize)]
struct RevisionForm {
    revision: String,
}

#[derive(Deserialize)]
struct MessageForm {
    revision: String,
    message: String,
}

#[derive(Deserialize)]
struct ModelForm {
    revision: String,
    provider: String,
    model: String,
    thinking: String,
}

#[derive(Deserialize)]
struct PresetForm {
    revision: String,
    preset: String,
}

#[derive(Deserialize)]
struct ProjectForm {
    revision: String,
    project: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CatalogueQuery {
    project: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ObserveQuery {
    job: String,
    cursor: String,
}

async fn catalogue(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Query(query): Query<CatalogueQuery>,
) -> AppResult<Response> {
    let filter = if query.project.is_empty() {
        None
    } else {
        ProjectId::parse(&query.project).filter(|project| state.projects.get(project).is_some())
    };
    let error = if query.project.is_empty() || filter.is_some() {
        ""
    } else {
        "Choose a project from the catalogue."
    };
    render_catalogue(&state, graft, filter, error)
}

async fn new_conversation(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    render_form_page(
        &state,
        graft,
        PatchStatus::Ok,
        ConversationFormView::new("", ""),
    )
}

async fn create(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Form(form): Form<ConversationForm>,
) -> AppResult<Response> {
    match state.conversations.create(form.title.clone()) {
        Ok(record) => Ok(responses::command_navigation(&conversation_path(&record))),
        Err(
            error @ (ConversationError::Random
            | ConversationError::Persist
            | ConversationError::Corrupt),
        ) => Err(AppError::new("store conversation", error)),
        Err(error) => render_form_command(
            graft,
            status_for(error),
            ConversationFormView::new(&form.title, error.message()),
        ),
    }
}

async fn detail(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: GraftRequest,
    Path(conversation_id): Path<String>,
    Query(query): Query<ObserveQuery>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::request_navigation(graft, "/conversations"));
    };
    if graft == GraftRequest::Patch && !query.job.is_empty() {
        return observe_message(state, session.0, record, query);
    }
    render_detail(
        &state,
        session.0,
        graft,
        PatchStatus::Ok,
        detail_view(&state, session.0, &record, &record.title, ""),
    )
}

async fn send_message(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<MessageForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    if record.active_job.is_some() {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                ConversationError::Active.message(),
            ),
        );
    }
    let Some(model) = effective_model(&state, &record) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                ConversationError::Selection.message(),
            ),
        );
    };
    if let Err(error) = valid_selection(&state, &model.selection) {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, error),
        );
    }
    let Some(connection) = state.vault.connection_for(&model.selection) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose a stored provider.",
            ),
        );
    };
    let job = match state.sessions.begin_conversation_job(
        &session.0,
        record.id,
        record.messages.len() + 1,
    ) {
        Ok(job) => job,
        Err(_) => {
            return render_detail_command(
                graft,
                PatchStatus::Conflict,
                detail_view(
                    &state,
                    session.0,
                    &record,
                    &record.title,
                    "Another command is active in this browser session.",
                ),
            );
        }
    };
    let started = state.conversations.begin_message_with_model(
        &record.id,
        revision,
        model,
        job.id(),
        form.message,
    );
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            state
                .sessions
                .finish_conversation_job(&session.0, record.id, job.id());
            let latest = state.conversations.get(&record.id).unwrap_or(record);
            return render_detail_command(
                graft,
                status_for(error),
                detail_view(&state, session.0, &latest, &latest.title, error.message()),
            );
        }
    };
    let view = detail_view(&state, session.0, &started, &started.title, "");
    let run_state = state.clone();
    tokio::spawn(job::run(
        run_state, session.0, started.id, started, connection, job,
    ));
    render_detail_command(graft, PatchStatus::Ok, view)
}

async fn cancel_message(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<ObserveQuery>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(job_id) = JobId::parse(&form.job) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(job) = state.sessions.conversation_job(record.id, job_id) else {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "This reply is no longer active.",
            ),
        );
    };
    job.request_cancel();
    render_detail_command(
        graft,
        PatchStatus::Ok,
        detail_view(&state, session.0, &record, &record.title, ""),
    )
}

fn observe_message(
    state: AppState,
    session: crate::sessions::SessionId,
    record: ConversationRecord,
    query: ObserveQuery,
) -> AppResult<Response> {
    if let Some(job) =
        JobId::parse(&query.job).and_then(|id| state.sessions.conversation_job(record.id, id))
    {
        let cursor = query
            .cursor
            .parse::<u64>()
            .unwrap_or(0)
            .min(job.latest_seq());
        return Ok(job::observe_response(
            state, record.id, session, job, cursor,
        ));
    }
    render_detail(
        &state,
        session,
        GraftRequest::Patch,
        PatchStatus::Ok,
        detail_view(&state, session, &record, &record.title, ""),
    )
}

async fn select_model(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<ModelForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(provider) = ProviderKind::parse(form.provider.trim()) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose a stored provider.",
            ),
        );
    };
    let has_thinking = !form.thinking.trim().is_empty();
    let thinking = if has_thinking {
        ThinkingEffort::new(form.thinking)
    } else {
        None
    };
    if has_thinking && thinking.is_none() {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose an available thinking effort.",
            ),
        );
    }
    let Some(selection) = ModelSelection::new(provider, form.model, thinking) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Enter a valid model name.",
            ),
        );
    };
    if let Err(error) = valid_selection(&state, &selection) {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, error),
        );
    }
    match state
        .conversations
        .select_model(&record.id, revision, selection)
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store model selection", error))
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn apply_preset(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<PresetForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(preset) = AgentId::parse(&form.preset).and_then(|id| state.agents.get(&id)) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose an available preset.",
            ),
        );
    };
    let Some(selection) = preset
        .selection
        .clone()
        .or_else(|| effective_model(&state, &record).map(|model| model.selection))
    else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose a model before you apply a preset without a model preference.",
            ),
        );
    };
    if let Err(error) = valid_selection(&state, &selection) {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, error),
        );
    }
    match state
        .conversations
        .apply_preset(&record.id, revision, &preset, selection)
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store conversation preset", error))
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn attach_project(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<ProjectForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    if state.sessions.busy(&session.0) {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Another command is active in this browser session.",
            ),
        );
    }
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(project) =
        ProjectId::parse(&form.project).filter(|project| state.projects.get(project).is_some())
    else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose a project from the catalogue.",
            ),
        );
    };
    match state
        .conversations
        .attach_project(&record.id, revision, project)
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store project context", error))
        }
        Err(ConversationError::Conflict) => {
            let latest = state.conversations.get(&record.id).unwrap_or(record);
            render_detail_command(
                graft,
                PatchStatus::Conflict,
                detail_view(
                    &state,
                    session.0,
                    &latest,
                    &latest.title,
                    ConversationError::Conflict.message(),
                ),
            )
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn detach_project(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path((conversation_id, project_id)): Path<(String, String)>,
    Form(form): Form<RevisionForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    if state.sessions.busy(&session.0) {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Another command is active in this browser session.",
            ),
        );
    }
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(project) = ProjectId::parse(&project_id) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "That project reference is invalid.",
            ),
        );
    };
    match state
        .conversations
        .detach_project(&record.id, revision, project)
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store project context", error))
        }
        Err(ConversationError::Conflict) => {
            let latest = state.conversations.get(&record.id).unwrap_or(record);
            render_detail_command(
                graft,
                PatchStatus::Conflict,
                detail_view(
                    &state,
                    session.0,
                    &latest,
                    &latest.title,
                    ConversationError::Conflict.message(),
                ),
            )
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn rename_conversation(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<RenameForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &form.title, REVISION_MESSAGE),
        );
    };
    match state
        .conversations
        .rename(&record.id, revision, form.title.clone())
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(
            error @ (ConversationError::Random
            | ConversationError::Persist
            | ConversationError::Corrupt),
        ) => Err(AppError::new("store conversation", error)),
        Err(ConversationError::Missing) => Ok(responses::command_navigation("/conversations")),
        Err(ConversationError::Conflict) => {
            let latest = state.conversations.get(&record.id).unwrap_or(record);
            render_detail_command(
                graft,
                PatchStatus::Conflict,
                detail_view(
                    &state,
                    session.0,
                    &latest,
                    &latest.title,
                    ConversationError::Conflict.message(),
                ),
            )
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &form.title, error.message()),
        ),
    }
}

async fn delete_conversation(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<RevisionForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    match state.conversations.delete(&record.id, revision) {
        Ok(()) | Err(ConversationError::Missing) => {
            Ok(responses::command_navigation("/conversations"))
        }
        Err(
            error @ (ConversationError::Random
            | ConversationError::Persist
            | ConversationError::Corrupt),
        ) => Err(AppError::new("store conversation", error)),
        Err(ConversationError::Conflict) => {
            let latest = state.conversations.get(&record.id).unwrap_or(record);
            render_detail_command(
                graft,
                PatchStatus::Conflict,
                detail_view(
                    &state,
                    session.0,
                    &latest,
                    &latest.title,
                    ConversationError::Conflict.message(),
                ),
            )
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

fn effective_model(
    state: &AppState,
    record: &ConversationRecord,
) -> Option<ConversationModelConfiguration> {
    record.model.clone().or_else(|| {
        state
            .vault
            .desk_providers()
            .into_iter()
            .find(|provider| provider.selected)
            .map(|connection| {
                ConversationModelConfiguration::direct(ModelSelection {
                    provider: connection.kind,
                    thinking: state.models_dev.effective_effort(
                        connection.kind,
                        &connection.model,
                        connection.thinking.as_ref(),
                    ),
                    model: connection.model,
                })
            })
    })
}

fn valid_selection(state: &AppState, selection: &ModelSelection) -> Result<(), &'static str> {
    if !state.vault.contains(selection.provider) {
        return Err("Choose a stored provider.");
    }
    if state
        .models_dev
        .model(selection.provider, &selection.model)
        .is_none()
    {
        return Err("Choose an available model.");
    }
    match selection.thinking.as_ref() {
        Some(effort)
            if !state
                .models_dev
                .supports(selection.provider, &selection.model, effort) =>
        {
            Err("Choose an available thinking effort.")
        }
        None if !state
            .models_dev
            .efforts(selection.provider, &selection.model)
            .is_empty() =>
        {
            Err("Choose an available thinking effort.")
        }
        _ => Ok(()),
    }
}

fn detail_view(
    state: &AppState,
    session: crate::sessions::SessionId,
    record: &ConversationRecord,
    title: &str,
    error: &'static str,
) -> ConversationDetailView {
    let snapshot = record
        .active_job
        .and_then(|job_id| state.sessions.conversation_job(record.id, job_id))
        .map(|job| job.snapshot());
    let error = if error.is_empty()
        && snapshot
            .as_ref()
            .is_some_and(|job| job.status == crate::sessions::JobStatus::Failed)
    {
        "The reply could not be stored. Restore access to local data, then restart Power Plant."
    } else {
        error
    };
    ConversationDetailView::from_record(
        record,
        ModelSources {
            vault: &state.vault,
            models: &state.models_dev,
            projects: &state.projects.list(),
        },
        &state.agents.list(),
        snapshot.as_ref(),
        state.sessions.busy(&session) || record.active_job.is_some(),
        title,
        error,
    )
}

fn load_conversation(state: &AppState, raw: &str) -> Option<ConversationRecord> {
    ConversationId::parse(raw).and_then(|id| state.conversations.get(&id))
}
fn parse_revision(raw: &str) -> Option<u32> {
    raw.parse().ok().filter(|revision| *revision > 0)
}
fn conversation_path(record: &ConversationRecord) -> String {
    format!("/conversations/{}", record.id.as_hex())
}
fn status_for(error: ConversationError) -> PatchStatus {
    match error {
        ConversationError::Conflict | ConversationError::Missing | ConversationError::Active => {
            PatchStatus::Conflict
        }
        _ => PatchStatus::UnprocessableEntity,
    }
}
fn render_catalogue(
    state: &AppState,
    graft: GraftRequest,
    filter: Option<ProjectId>,
    error: &'static str,
) -> AppResult<Response> {
    render_page(
        state,
        graft,
        if error.is_empty() {
            PatchStatus::Ok
        } else {
            PatchStatus::UnprocessableEntity
        },
        page::CATALOGUE_TITLE,
        &CatalogueView::from_records(
            &state.conversations.list(),
            &state.projects.list(),
            filter,
            error,
        ),
    )
}

fn render_form_page(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    view: ConversationFormView,
) -> AppResult<Response> {
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(page::NEW_TITLE, state, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            page::NEW_TITLE,
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "conversation-form",
            &view.contents(),
        )?),
    }
}
fn render_form_command(
    _graft: PatchGraft,
    status: PatchStatus,
    view: ConversationFormView,
) -> AppResult<Response> {
    Ok(hypergraft::outcome::children_patch(
        status,
        "conversation-form",
        &view.contents(),
    )?)
}
fn render_detail(
    state: &AppState,
    _session: crate::sessions::SessionId,
    graft: GraftRequest,
    status: PatchStatus,
    view: ConversationDetailView,
) -> AppResult<Response> {
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(&view.document_title, state, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            &view.document_title,
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "conversation-detail",
            &view.contents(),
        )?),
    }
}
fn render_detail_command(
    _graft: PatchGraft,
    status: PatchStatus,
    view: ConversationDetailView,
) -> AppResult<Response> {
    Ok(hypergraft::PatchSet::new()
        .title(&view.document_title)
        .with_children("conversation-detail", &view.contents())?
        .respond(status)?)
}
fn render_page<T: askama::Template>(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    title: &str,
    view: &T,
) -> AppResult<Response> {
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(title, state, view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(title, "chat-main", view)?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "chat-main",
            view,
        )?),
    }
}
