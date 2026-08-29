mod forms;
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

use hypergraft::{CommandGraft, GraftRequest, PatchSet, PatchStatus};

use crate::{
    agents::{AgentId, AgentRecord, DirectoryPolicy},
    error::AppResult,
    responses,
    sandbox::GuestAccess,
    sessions::{self, BeginTurnError, JobIdError, OptionalSession, SessionId, SessionSnapshot},
    state::AppState,
    workflows::{
        self, ResolveWorkflowError, WorkflowJob, WorkflowRun, WorkflowSelection,
        definition_fits_agent,
    },
};

use self::{
    forms::{ChatForm, CursorError, ModelForm, ObserveQuery},
    job::{observe_response, user_transcript_patch},
    page::{ChatViewModel, JobObserveContents, TranscriptContents},
};

pub(crate) use job::{AgentOutcome, AgentRunSpec, run_agent_action};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/model", get(refresh_model_options).post(update_model))
        .route("/agents/{agent_id}", get(show).post(send))
        .route("/agents/{agent_id}/jobs/{job_id}/cancel", post(cancel))
}

fn parse_agent(raw: &str) -> Option<AgentId> {
    AgentId::parse(raw)
}

async fn require_chat(
    state: &AppState,
    session: Option<SessionId>,
    agent_id: &str,
    graft: impl Into<hypergraft::GraftRequest>,
) -> Result<(SessionId, AgentRecord, SessionSnapshot), Response> {
    let graft = graft.into();
    let Some(session) = session else {
        return Err(responses::graft_redirect(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Err(responses::graft_redirect(graft, "/connect"));
    }
    let Some(agent_id) = parse_agent(agent_id) else {
        return Err(responses::graft_redirect(graft, "/agents"));
    };
    let Some(record) = state.agents.get(&agent_id) else {
        return Err(responses::graft_redirect(graft, "/agents"));
    };
    let Some(snapshot) = state.sessions.snapshot(&session, &agent_id) else {
        return Err(responses::graft_redirect(graft, "/connect"));
    };
    Ok((session, record, snapshot))
}

async fn show(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
    Path(agent_id): Path<String>,
    Query(query): Query<ObserveQuery>,
) -> AppResult<Response> {
    let (session, record, snapshot) = match require_chat(&state, session, &agent_id, graft).await {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    match graft {
        GraftRequest::Document => render_document(
            &state,
            PatchStatus::Ok,
            view(&state, &record, &snapshot, "", "", &query.workflow).await,
        ),
        GraftRequest::Navigation => navigate_page(
            &state,
            &view(&state, &record, &snapshot, "", "", &query.workflow).await,
        ),
        GraftRequest::Patch => observe(&state, &session, &record, &snapshot, query).await,
    }
}

async fn send(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Path(agent_id): Path<String>,
    Form(form): Form<ChatForm>,
) -> AppResult<Response> {
    let (session, record, snapshot) = match require_chat(&state, session, &agent_id, graft).await {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let Some(connection) = state.vault.selected_connection() else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };

    if !form.is_bounded() {
        return reject_chat_input(
            &state,
            graft,
            &record,
            &snapshot,
            "Enter a message.",
            &form.message,
        )
        .await;
    }
    let selection = match form.workflow_selection() {
        Ok(selection) => selection,
        Err(_) => {
            return reject_chat_input(
                &state,
                graft,
                &record,
                &snapshot,
                "Choose a workflow.",
                &form.message,
            )
            .await;
        }
    };
    if snapshot.session_busy {
        return reject_parallel_command(&state, graft, &record, &snapshot).await;
    }

    let Ok(execution) = state.workflow_execution.acquire() else {
        return reject_chat_input(
            &state,
            graft,
            &record,
            &snapshot,
            "Wait until the current workflow finishes.",
            &form.message,
        )
        .await;
    };
    let Ok(lease) = state.agent_leases.acquire(record.id) else {
        return reject_chat_input(
            &state,
            graft,
            &record,
            &snapshot,
            "Wait until this agent finishes.",
            &form.message,
        )
        .await;
    };
    let Some(latest) = state.agents.get(&record.id) else {
        return Ok(responses::graft_redirect(graft, "/agents"));
    };
    if latest.revision != record.revision {
        return reject_chat_input(
            &state,
            graft,
            &latest,
            &snapshot,
            "The agent configuration changed. Try again.",
            &form.message,
        )
        .await;
    }
    let record = latest;
    let policy = DirectoryPolicy::from_record(&record);
    if policy.confirm_hosts().is_err() {
        return reject_chat_input(
            &state,
            graft,
            &record,
            &snapshot,
            "A granted directory is no longer at the saved path.",
            &form.message,
        )
        .await;
    }
    let access = GuestAccess::from_connection(&connection);

    let resolved = match state.workflows.resolve(&selection) {
        Ok(resolved) => resolved,
        Err(error) => {
            let status = match error {
                ResolveWorkflowError::Missing | ResolveWorkflowError::Changed => {
                    PatchStatus::Conflict
                }
                ResolveWorkflowError::Invalid => PatchStatus::UnprocessableEntity,
            };
            return reject_chat_selection(
                &state,
                graft,
                &record,
                &snapshot,
                error.message(),
                &form.message,
                status,
            )
            .await;
        }
    };
    let directories: Vec<(String, crate::agents::AccessMode)> = record
        .directories
        .iter()
        .map(|grant| (grant.alias.clone(), grant.access))
        .collect();
    if !definition_fits_agent(&resolved.pinned.definition, &record.tools, &directories) {
        return reject_chat_input(
            &state,
            graft,
            &record,
            &snapshot,
            "That workflow needs access this agent does not allow.",
            &form.message,
        )
        .await;
    }
    let environments = match workflows::resolve_environments(
        &resolved.pinned.definition,
        &state.environments,
        &state.environment_snapshots,
    )
    .await
    {
        Ok(environments) => environments,
        Err(error) => {
            return reject_chat_input(
                &state,
                graft,
                &record,
                &snapshot,
                error.message(),
                &form.message,
            )
            .await;
        }
    };

    let message = form.message.trim().to_owned();
    let run_id = workflows::RunId::generate()
        .map_err(|error| crate::error::AppError::new("create workflow run identifier", error))?;
    let workflow_name = resolved.pinned.definition.name().to_owned();
    let step_name = resolved
        .pinned
        .definition
        .step(resolved.pinned.definition.first_step())
        .map(|step| step.name.clone())
        .unwrap_or_default();
    let pinned = resolved.pinned;
    let started = match state
        .sessions
        .begin_turn(&session, record.id, run_id, message)
    {
        Ok(started) => started,
        Err(BeginTurnError::MissingSession) => {
            return Ok(responses::graft_redirect(graft, "/connect"));
        }
        Err(BeginTurnError::Conflict) => {
            let Some(latest) = state.sessions.snapshot(&session, &record.id) else {
                return Ok(responses::graft_redirect(graft, "/connect"));
            };
            return reject_parallel_command(&state, graft, &record, &latest).await;
        }
        Err(BeginTurnError::JobId) => {
            return Err(crate::error::AppError::new(
                "create job identifier",
                JobIdError::RandomUnavailable,
            ));
        }
    };
    let run = WorkflowRun::create(run_id, workflows::now_ms(), pinned, environments);
    if let Err(error) = state.workflow_runs.create(run) {
        let _ = state
            .sessions
            .rollback_turn(&session, &record.id, &started.job.id());
        return Err(crate::error::AppError::new("store workflow run", error));
    }
    state
        .sessions
        .set_preferred_workflow(&session, record.id, selection.workflow_id);
    started.job.set_workflow_name(workflow_name);
    started.job.set_step_label(step_name);
    tokio::spawn(workflows::execute_run(
        state.clone(),
        WorkflowJob {
            run_id,
            session_id: session,
            agent_id: record.id,
            connection,
            sandbox: state.sandboxes.run_handle(run_id),
            host_policy: policy,
            turns: started.turns.clone(),
            job: started.job.clone(),
            access,
        },
        lease,
        execution,
    ));

    match graft {
        CommandGraft::Document => {
            let Some(latest) = state.sessions.snapshot(&session, &record.id) else {
                return Ok(responses::graft_redirect(graft, "/connect"));
            };
            render_document(
                &state,
                PatchStatus::Ok,
                view(&state, &record, &latest, "", "", "").await,
            )
        }
        CommandGraft::Patch => accept_job_patch(
            &started.turns,
            &started.job.id().as_hex(),
            &record.id.as_hex(),
            &started.job.run_id().as_hex(),
            &started.job.step_label(),
            &started.job.workflow_name(),
        ),
    }
}

async fn refresh_model_options(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    match graft {
        GraftRequest::Patch => {
            let view = desk_view(&state, session).await;
            Ok(hypergraft::outcome::children_patch(
                PatchStatus::Ok,
                "desk-model-catalogue",
                &view.desk_model_catalogue(),
            )?)
        }
        GraftRequest::Document | GraftRequest::Navigation => {
            Ok(responses::graft_redirect(graft, "/"))
        }
    }
}

async fn update_model(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<ModelForm>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }
    if state.sessions.busy(&session) {
        let view = desk_view(&state, session).await;
        return reject_model_view(&state, graft, view, "Wait until this reply finishes.").await;
    }
    if form.wants_favourite_toggle() {
        return toggle_favourite(&state, graft, session, &form).await;
    }

    match form.validate(|kind| state.vault.contains(kind)) {
        Ok((kind, model)) => {
            let model = submitted_model(&state, &form, kind, model);
            state
                .vault
                .select(kind, model)
                .map_err(|error| crate::error::AppError::new("store model", error))?;
        }
        Err(forms::ModelError::Provider) => {
            let view = desk_view(&state, session).await;
            return reject_model_view(&state, graft, view, "Choose a stored provider.").await;
        }
        Err(forms::ModelError::Model) => {
            let view = desk_view(&state, session).await;
            return reject_model_view(&state, graft, view, "That model name is too long.").await;
        }
    }

    let view = desk_view(&state, session).await;
    match graft {
        CommandGraft::Document => render_document(&state, PatchStatus::Ok, view),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "desk-settings",
            &view.desk_settings(),
        )?),
    }
}

async fn cancel(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Path((agent_id, job_id)): Path<(String, String)>,
) -> AppResult<Response> {
    let (session, record, _) = match require_chat(&state, session, &agent_id, graft).await {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    if let Some(id) = sessions::JobId::parse(&job_id)
        && let Some(job) = state.sessions.job(&session, &record.id, &id)
    {
        job.request_cancel();
    }
    let Some(latest) = state.sessions.snapshot(&session, &record.id) else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    match graft {
        CommandGraft::Document => render_document(
            &state,
            PatchStatus::Ok,
            view(&state, &record, &latest, "", "", "").await,
        ),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "job-observe",
            &view(&state, &record, &latest, "", "", "")
                .await
                .job_observe(),
        )?),
    }
}

async fn toggle_favourite(
    state: &AppState,
    graft: CommandGraft,
    session: SessionId,
    form: &ModelForm,
) -> AppResult<Response> {
    match form.validate_favourite(|kind| state.vault.contains(kind)) {
        Ok((kind, model)) => {
            let model = submitted_model(state, form, kind, model);
            match state.vault.toggle_favourite(kind, &model) {
                Ok(_) => {}
                Err(crate::vault::FavouriteError::Provider) => {
                    let view = desk_view(state, session).await;
                    return reject_model_view(state, graft, view, "Choose a stored provider.")
                        .await;
                }
                Err(crate::vault::FavouriteError::Full) => {
                    let view = desk_view(state, session).await;
                    return reject_model_view(state, graft, view, "The favourites list is full.")
                        .await;
                }
                Err(crate::vault::FavouriteError::Persist(error)) => {
                    return Err(crate::error::AppError::new("store favourite", error));
                }
            }
        }
        Err(forms::ModelError::Provider) => {
            let view = desk_view(state, session).await;
            return reject_model_view(state, graft, view, "Choose a stored provider.").await;
        }
        Err(forms::ModelError::Model) => {
            let view = desk_view(state, session).await;
            return reject_model_view(state, graft, view, "Choose a model.").await;
        }
    }
    let view = desk_view(state, session).await;
    match graft {
        CommandGraft::Document => render_document(state, PatchStatus::Ok, view),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "desk-model-catalogue",
            &view.desk_model_catalogue(),
        )?),
    }
}

fn submitted_model(
    state: &AppState,
    form: &ModelForm,
    kind: crate::providers::ProviderKind,
    model: String,
) -> String {
    if form.provider_model_synced {
        return model;
    }
    let providers = state.vault.desk_providers();
    let Some(selected) = providers.iter().find(|provider| provider.selected) else {
        return model;
    };
    if selected.kind == kind {
        return model;
    }
    providers
        .iter()
        .find(|provider| provider.kind == kind)
        .map(|provider| provider.model.clone())
        .unwrap_or(model)
}

async fn observe(
    state: &AppState,
    session: &SessionId,
    record: &AgentRecord,
    snapshot: &SessionSnapshot,
    query: ObserveQuery,
) -> AppResult<Response> {
    let cursor = match query.cursor() {
        Ok(cursor) => cursor,
        Err(CursorError::Malformed | CursorError::Excessive) => {
            return Ok(hypergraft::outcome::children_patch(
                PatchStatus::UnprocessableEntity,
                "job-observe",
                &view(state, record, snapshot, "", "", "")
                    .await
                    .job_observe_with("That cursor is not valid."),
            )?);
        }
    };
    let Some(job_id) = query.job_id() else {
        if !query.workflow.trim().is_empty() {
            return Ok(hypergraft::outcome::children_patch(
                PatchStatus::Ok,
                "composer",
                &view(state, record, snapshot, "", "", &query.workflow)
                    .await
                    .composer(),
            )?);
        }
        return refresh_composer(state, record, snapshot).await;
    };
    let Some(job) = state.sessions.job(session, &record.id, &job_id) else {
        return refresh_composer(state, record, snapshot).await;
    };
    Ok(observe_response(job, cursor, record.id.as_hex()))
}

async fn refresh_composer(
    state: &AppState,
    record: &AgentRecord,
    snapshot: &SessionSnapshot,
) -> AppResult<Response> {
    Ok(hypergraft::outcome::children_patch(
        PatchStatus::Ok,
        "job-observe",
        &view(state, record, snapshot, "", "", "")
            .await
            .job_observe(),
    )?)
}

fn accept_job_patch(
    turns: &[crate::providers::ChatTurn],
    job_id: &str,
    agent_id: &str,
    run_id: &str,
    run_step: &str,
    workflow_name: &str,
) -> AppResult<Response> {
    let mut patches = user_transcript_patch(turns)?;
    patches.children(
        "job-observe",
        &JobObserveContents::observing(
            job_id,
            0,
            "Writing",
            "",
            agent_id,
            run_id,
            run_step,
            workflow_name,
        ),
    )?;
    Ok(patches.respond(PatchStatus::Ok)?)
}

async fn reject_parallel_command(
    state: &AppState,
    graft: CommandGraft,
    record: &AgentRecord,
    session: &SessionSnapshot,
) -> AppResult<Response> {
    const MESSAGE: &str = "Wait until this reply finishes.";
    match graft {
        CommandGraft::Document => render_document(
            state,
            PatchStatus::Conflict,
            view(state, record, session, MESSAGE, "", "").await,
        ),
        CommandGraft::Patch => {
            let view = view(state, record, session, "", "", "").await;
            let mut patches = PatchSet::new();
            patches.children("transcript", &TranscriptContents { turns: &view.turns })?;
            patches.children("job-observe", &view.job_observe_with(MESSAGE))?;
            Ok(patches.respond(PatchStatus::Conflict)?)
        }
    }
}

async fn reject_chat_input(
    state: &AppState,
    graft: CommandGraft,
    record: &AgentRecord,
    session: &SessionSnapshot,
    message: &'static str,
    draft: &str,
) -> AppResult<Response> {
    reject_chat_selection(
        state,
        graft,
        record,
        session,
        message,
        draft,
        PatchStatus::UnprocessableEntity,
    )
    .await
}

async fn reject_chat_selection(
    state: &AppState,
    graft: CommandGraft,
    record: &AgentRecord,
    session: &SessionSnapshot,
    message: &'static str,
    draft: &str,
    status: PatchStatus,
) -> AppResult<Response> {
    let mut page = view(state, record, session, message, "", "").await;
    page.draft_message = draft.trim().to_owned();
    match graft {
        CommandGraft::Document => render_document(state, status, page),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "composer",
            &page.composer(),
        )?),
    }
}

async fn reject_model_view(
    state: &AppState,
    graft: CommandGraft,
    view: ChatViewModel,
    message: &'static str,
) -> AppResult<Response> {
    let mut view = view;
    view.desk_error = message;
    match graft {
        CommandGraft::Document => render_document(state, PatchStatus::UnprocessableEntity, view),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::UnprocessableEntity,
            "desk-settings",
            &view.desk_settings(),
        )?),
    }
}

async fn view(
    state: &AppState,
    record: &AgentRecord,
    session: &SessionSnapshot,
    error: &'static str,
    desk_error: &'static str,
    workflow_query: &str,
) -> ChatViewModel {
    let mut page = ChatViewModel::from_session(
        record,
        session,
        &state.vault,
        &state.models,
        error,
        desk_error,
    );
    attach_workflow_ui(state, session, &mut page, workflow_query);
    attach_environment_preview(state, &mut page).await;
    page
}

async fn attach_environment_preview(state: &AppState, page: &mut ChatViewModel) {
    page.preview_ready = false;
    page.environment_preview.clear();
    page.environment_preview_error = "";
    let Some(option) = page.workflow_options.iter().find(|option| option.selected) else {
        return;
    };
    let Some(selection) = WorkflowSelection::parse(&option.token) else {
        page.environment_preview_error = "Choose a workflow.";
        return;
    };
    let Ok(resolved) = state.workflows.resolve(&selection) else {
        page.environment_preview_error = "That workflow is not valid. Choose another.";
        return;
    };
    match workflows::preview_environments(
        &resolved.pinned.definition,
        &state.environments,
        &state.environment_snapshots,
    )
    .await
    {
        Ok(preview) => {
            for environment in &preview.environments {
                page.environment_preview.push(page::PreviewLine {
                    text: format!(
                        "{} · preparation {} · {}",
                        environment.name,
                        environment.preparation_ordinal,
                        environment.snapshot_short
                    ),
                });
            }
            for step in preview.steps {
                page.environment_preview.push(page::PreviewLine {
                    text: format!(
                        "{} · {} · preparation {} · {}",
                        step.step,
                        step.environment_name,
                        step.preparation_ordinal,
                        step.snapshot_short
                    ),
                });
            }
            page.preview_ready = true;
        }
        Err(error) => page.environment_preview_error = error.message(),
    }
}

fn attach_workflow_ui(
    state: &AppState,
    session: &SessionSnapshot,
    page: &mut ChatViewModel,
    workflow_query: &str,
) {
    let records = state.workflows.list();
    if records.is_empty() {
        if session.preferred_workflow.is_some()
            && let Some(agent) = crate::agents::AgentId::parse(&page.agent_id)
        {
            state.sessions.clear_preferred_workflow(&session.id, &agent);
        }
        page.workflow_options = Vec::new();
        page.workflow_empty = true;
    } else {
        let mut preferred = session.preferred_workflow;
        if let Some(id) = preferred
            && state.workflows.get(&id).is_none()
        {
            if let Some(agent) = crate::agents::AgentId::parse(&page.agent_id) {
                state.sessions.clear_preferred_workflow(&session.id, &agent);
            }
            preferred = None;
        }
        let queried = WorkflowSelection::parse(workflow_query.trim()).map(|item| item.workflow_id);
        let selected = queried.or(preferred).or_else(|| {
            if records.len() == 1 {
                Some(records[0].id)
            } else {
                None
            }
        });
        page.workflow_empty = false;
        page.workflow_options = records
            .iter()
            .map(|record| page::WorkflowOption {
                token: WorkflowSelection {
                    workflow_id: record.id,
                    definition_version: record.definition_version,
                }
                .as_token(),
                label: record.definition.name().to_owned(),
                selected: selected == Some(record.id),
            })
            .collect();
    }
}

async fn desk_view(state: &AppState, session: SessionId) -> ChatViewModel {
    let record = state.agents.list().into_iter().next();
    let Some(record) = record else {
        return ChatViewModel::desk_only(
            &state.vault,
            &state.models,
            state.sessions.busy(&session),
            "",
        );
    };
    let snapshot = state
        .sessions
        .snapshot(&session, &record.id)
        .expect("live session");
    view(state, &record, &snapshot, "", "", "").await
}

fn navigate_page(state: &AppState, view: &ChatViewModel) -> AppResult<Response> {
    match hypergraft::outcome::page_patch(page::DOCUMENT_TITLE, "chat-main", view) {
        Ok(response) => Ok(response),
        Err(error) if error.kind() == hypergraft::PatchBuildErrorKind::ResponseLimit => {
            crate::error::trace_patch_build_failure("construct chat page navigation patch", &error);
            responses::chat_page_response(page::DOCUMENT_TITLE, &state.assets, view)
        }
        Err(error) => Err(error.into()),
    }
}

fn render_document(
    state: &AppState,
    status: PatchStatus,
    view: ChatViewModel,
) -> AppResult<Response> {
    let mut response = responses::chat_page_response(page::DOCUMENT_TITLE, &state.assets, &view)?;
    responses::apply_patch_status(&mut response, status);
    Ok(response)
}
