mod forms;
mod job;
mod page;

#[cfg(test)]
mod tests;

use std::time::Duration;

use axum::{
    Extension, Form, Router,
    extract::{Query, State},
    response::Response,
    routing::get,
};

use hypergraft::{GraftRequest, PatchGraft, PatchSet, PatchStatus};

use crate::{
    agents::AgentRecord,
    environments::EnvironmentCatalogue,
    error::AppResult,
    projects::{ProjectId, ProjectRecord, eligibility, eligible_agents},
    responses,
    sessions::{
        ConversationKey, JobSnapshot, JobStatus, OptionalSession, SessionId, SessionSnapshot,
    },
    state::AppState,
    workflows::{self, RunKind, WorkflowSelection},
};

use self::{
    forms::{CursorError, ModelForm},
    job::{observe_response, user_transcript_patch},
    page::{ChatViewModel, JobObserveContents, TranscriptContents},
};

pub(crate) use forms::{ChatForm, DeskMode, ObserveQuery};
pub(crate) use job::{AgentOutcome, AgentRunSpec, bound_reply, run_agent_action};

const SANDBOX_HOLD: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(1)
};

#[derive(Clone, Copy)]
pub(crate) struct DeskPage<'a> {
    pub(crate) project: &'a ProjectRecord,
    pub(crate) agent: &'a AgentRecord,
    pub(crate) eligible: &'a [AgentRecord],
    pub(crate) snapshot: &'a SessionSnapshot,
}

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/model", get(refresh_model_options).post(update_model))
}

pub(super) fn live_router() -> hypergraft::live::LiveRouter<AppState> {
    hypergraft::live::LiveRouter::new()
        .route("/model", model_live)
        .expect("live projection paths are unique")
}

#[derive(Default, serde::Deserialize)]
struct ModelQuery {
    #[serde(default)]
    project: String,
    #[serde(default)]
    agent: String,
}

struct ResolvedDesk {
    project: ProjectRecord,
    agent: AgentRecord,
    eligible: Vec<AgentRecord>,
    snapshot: SessionSnapshot,
}

async fn model_live(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Query(query): Query<ModelQuery>,
) -> Result<hypergraft::live::LiveProjection<SessionId>, hypergraft::live::LiveReject> {
    let invalidations = state.models.subscribe();
    let Some(project) = ProjectId::parse(query.project.trim()) else {
        return Err(hypergraft::live::LiveReject::Invalid);
    };
    let Some(agent) = crate::agents::AgentId::parse(query.agent.trim()) else {
        return Err(hypergraft::live::LiveReject::Invalid);
    };
    if !state.vault.has_providers()
        || resolved_desk(&state, session, &project.as_hex(), &agent.as_hex()).is_none()
    {
        return Err(hypergraft::live::LiveReject::Retire);
    }
    Ok(hypergraft::live::LiveProjection::new(
        hypergraft::live::broadcast_invalidations(invalidations),
        move |session| {
            let state = state.clone();
            async move { refresh_model_projection(&state, session, project, agent).await }
        },
    ))
}

async fn refresh_model_projection(
    state: &AppState,
    session: SessionId,
    project: ProjectId,
    agent: crate::agents::AgentId,
) -> Result<PatchSet, hypergraft::live::ProjectionError> {
    if !state.vault.has_providers() {
        return Err(hypergraft::live::ProjectionError::Retire);
    }
    let desk = resolved_desk(state, session, &project.as_hex(), &agent.as_hex())
        .ok_or(hypergraft::live::ProjectionError::Retire)?;
    let view = desk_view(state, &desk).await;
    let mut patches = PatchSet::new();
    patches
        .children("desk-model-catalogue", &view.desk_model_catalogue())
        .map_err(|_| hypergraft::live::ProjectionError::Retire)?;
    Ok(patches)
}

impl ResolvedDesk {
    fn page(&self) -> DeskPage<'_> {
        DeskPage {
            project: &self.project,
            agent: &self.agent,
            eligible: &self.eligible,
            snapshot: &self.snapshot,
        }
    }
}

async fn refresh_model_options(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
    Query(query): Query<ModelQuery>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::request_navigation(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Ok(responses::request_navigation(graft, "/connect"));
    }
    match graft {
        GraftRequest::Patch => {
            let Some(desk) = resolved_desk(&state, session, &query.project, &query.agent) else {
                return Ok(responses::request_navigation(graft, "/projects"));
            };
            let view = desk_view(&state, &desk).await;
            Ok(hypergraft::outcome::children_patch(
                PatchStatus::Ok,
                "desk-model-catalogue",
                &view.desk_model_catalogue(),
            )?)
        }
        GraftRequest::Document | GraftRequest::Navigation => {
            Ok(responses::request_navigation(graft, "/"))
        }
    }
}

async fn update_model(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: PatchGraft,
    Form(form): Form<ModelForm>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::request_navigation(graft, "/connect"));
    };
    if !state.vault.has_providers() {
        return Ok(responses::request_navigation(graft, "/connect"));
    }
    let Some(desk) = resolved_desk(&state, session, &form.project, &form.agent) else {
        return Ok(responses::request_navigation(graft, "/projects"));
    };
    if state.sessions.busy(&session) {
        let view = desk_view(&state, &desk).await;
        return reject_model_view(&state, graft, view, "Wait until this reply finishes.").await;
    }
    if form.wants_favourite_toggle() {
        return toggle_favourite(&state, graft, &desk, &form).await;
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
            let view = desk_view(&state, &desk).await;
            return reject_model_view(&state, graft, view, "Choose a stored provider.").await;
        }
        Err(forms::ModelError::Model) => {
            let view = desk_view(&state, &desk).await;
            return reject_model_view(&state, graft, view, "That model name is too long.").await;
        }
    }

    let view = desk_view(&state, &desk).await;
    Ok(hypergraft::outcome::children_patch(
        PatchStatus::Ok,
        "desk-settings",
        &view.desk_settings(),
    )?)
}

async fn toggle_favourite(
    state: &AppState,
    graft: PatchGraft,
    desk: &ResolvedDesk,
    form: &ModelForm,
) -> AppResult<Response> {
    match form.validate_favourite(|kind| state.vault.contains(kind)) {
        Ok((kind, model)) => {
            let model = submitted_model(state, form, kind, model);
            match state.vault.toggle_favourite(kind, &model) {
                Ok(_) => {}
                Err(crate::vault::FavouriteError::Provider) => {
                    let view = desk_view(state, desk).await;
                    return reject_model_view(state, graft, view, "Choose a stored provider.")
                        .await;
                }
                Err(crate::vault::FavouriteError::Full) => {
                    let view = desk_view(state, desk).await;
                    return reject_model_view(state, graft, view, "The favourites list is full.")
                        .await;
                }
                Err(crate::vault::FavouriteError::Persist(error)) => {
                    return Err(crate::error::AppError::new("store favourite", error));
                }
            }
        }
        Err(forms::ModelError::Provider) => {
            let view = desk_view(state, desk).await;
            return reject_model_view(state, graft, view, "Choose a stored provider.").await;
        }
        Err(forms::ModelError::Model) => {
            let view = desk_view(state, desk).await;
            return reject_model_view(state, graft, view, "Choose a model.").await;
        }
    }
    let view = desk_view(state, desk).await;
    Ok(hypergraft::outcome::children_patch(
        PatchStatus::Ok,
        "desk-model-catalogue",
        &view.desk_model_catalogue(),
    )?)
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

pub(crate) async fn observe(
    state: &AppState,
    session: &SessionId,
    page: DeskPage<'_>,
    query: ObserveQuery,
) -> AppResult<Response> {
    if !query.sandbox.trim().is_empty() {
        return observe_sandbox(state, session, page, &query).await;
    }
    let key = ConversationKey {
        project_id: page.project.id,
        agent_id: page.agent.id,
    };
    let cursor = match query.cursor() {
        Ok(cursor) => cursor,
        Err(CursorError::Malformed | CursorError::Excessive) => {
            return Ok(hypergraft::outcome::children_patch(
                PatchStatus::UnprocessableEntity,
                "job-observe",
                &view(state, page, "", "", "")
                    .await
                    .job_observe_with("That cursor is not valid."),
            )?);
        }
    };
    let Some(job_id) = query.job_id() else {
        if !query.workflow.trim().is_empty() {
            if let Some(selection) = WorkflowSelection::parse(query.workflow.trim())
                && state.workflows.resolve(&selection).is_ok()
            {
                state
                    .sessions
                    .set_preferred_workflow(session, key, selection.workflow_id);
            }
            let rendered = view(state, page, "", "", &query.workflow).await;
            return Ok(hypergraft::outcome::children_patch(
                PatchStatus::Ok,
                "composer",
                &rendered.composer(),
            )?);
        }
        return refresh_composer(state, page).await;
    };
    let Some(job) = state.sessions.job(session, &key, &job_id) else {
        return refresh_composer(state, page).await;
    };
    Ok(observe_response(
        state.clone(),
        job,
        cursor,
        crate::projects::desk_path(&page.project.id, &page.agent.id),
    ))
}

async fn observe_sandbox(
    state: &AppState,
    session: &SessionId,
    page: DeskPage<'_>,
    query: &ObserveQuery,
) -> AppResult<Response> {
    let Some(cursor) = EnvironmentCatalogue::parse_refresh_cursor(query.sandbox.trim()) else {
        let rendered = view(state, page, "", "", &query.workflow).await;
        return Ok(hypergraft::outcome::children_patch(
            PatchStatus::UnprocessableEntity,
            "sandbox-status",
            &rendered.sandbox_observe(),
        )?);
    };
    if !state.environments.cursor_is_stale(Some(cursor)) {
        state
            .environments
            .wait_while_current(cursor, SANDBOX_HOLD)
            .await;
    }
    let key = ConversationKey {
        project_id: page.project.id,
        agent_id: page.agent.id,
    };
    let snapshot = state
        .sessions
        .snapshot(session, &key)
        .unwrap_or_else(|| page.snapshot.clone());
    let rendered = view(
        state,
        DeskPage {
            project: page.project,
            agent: page.agent,
            eligible: page.eligible,
            snapshot: &snapshot,
        },
        "",
        "",
        &query.workflow,
    )
    .await;
    let mut patches = PatchSet::new();
    patches.children("sandbox-status", &rendered.sandbox_observe())?;
    patches.children("composer", &rendered.composer())?;
    Ok(patches.respond(PatchStatus::Ok)?)
}

async fn refresh_composer(state: &AppState, page: DeskPage<'_>) -> AppResult<Response> {
    Ok(hypergraft::outcome::children_patch(
        PatchStatus::Ok,
        "job-observe",
        &view(state, page, "", "", "").await.job_observe(),
    )?)
}

pub(crate) fn accept_job_patch(
    turns: &[crate::providers::ChatTurn],
    job_id: &str,
    desk_href: &str,
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
            run_step,
            "",
            desk_href,
            run_id,
            run_step,
            workflow_name,
        ),
    )?;
    Ok(patches.respond(PatchStatus::Ok)?)
}

pub(crate) async fn reject_parallel_command(
    state: &AppState,
    _graft: PatchGraft,
    page: DeskPage<'_>,
) -> AppResult<Response> {
    const MESSAGE: &str = "Wait until this reply finishes.";
    let view = view(state, page, "", "", "").await;
    let mut patches = PatchSet::new();
    patches.children("transcript", &TranscriptContents { turns: &view.turns })?;
    patches.children("job-observe", &view.job_observe_with(MESSAGE))?;
    Ok(patches.respond(PatchStatus::Conflict)?)
}

pub(crate) async fn reject_chat_input(
    state: &AppState,
    graft: PatchGraft,
    page: DeskPage<'_>,
    message: &'static str,
    draft: &str,
) -> AppResult<Response> {
    reject_chat_selection(
        state,
        graft,
        page,
        message,
        draft,
        PatchStatus::UnprocessableEntity,
    )
    .await
}

pub(crate) async fn reject_chat_selection(
    state: &AppState,
    _graft: PatchGraft,
    page: DeskPage<'_>,
    message: &'static str,
    draft: &str,
    status: PatchStatus,
) -> AppResult<Response> {
    let mut rendered = view(state, page, message, "", "").await;
    rendered.draft_message = draft.trim().to_owned();
    Ok(hypergraft::outcome::children_patch(
        status,
        "composer",
        &rendered.composer(),
    )?)
}

async fn reject_model_view(
    _state: &AppState,
    _graft: PatchGraft,
    view: ChatViewModel,
    message: &'static str,
) -> AppResult<Response> {
    let mut view = view;
    view.desk_error = message;
    Ok(hypergraft::outcome::children_patch(
        PatchStatus::UnprocessableEntity,
        "desk-settings",
        &view.desk_settings(),
    )?)
}

pub(crate) async fn view(
    state: &AppState,
    page: DeskPage<'_>,
    error: &'static str,
    desk_error: &'static str,
    workflow_query: &str,
) -> ChatViewModel {
    let mut rendered = ChatViewModel::from_session(
        page.agent,
        page.snapshot,
        &state.vault,
        &state.models,
        error,
        desk_error,
    )
    .with_project(page.project, page.agent, page.eligible);
    attach_workflow_ui(state, page.snapshot, &mut rendered, workflow_query);
    attach_environment_preview(state, &mut rendered).await;
    attach_sandbox_status(state, &mut rendered).await;
    if let Some(job) = page.snapshot.job.as_ref() {
        rendered.review_href = review_href_for(state, job);
        rendered.quick_task_finished = quick_task_finished(state, job);
    }
    rendered
}

pub(super) fn quick_task_finished(state: &AppState, job: &JobSnapshot) -> bool {
    job.status == JobStatus::Completed
        && state
            .workflow_runs
            .get(&job.run_id)
            .is_some_and(|run| run.kind == RunKind::QuickTask)
}

pub(super) fn review_href_for(state: &AppState, job: &JobSnapshot) -> String {
    if job.status != JobStatus::AwaitingDecision {
        return String::new();
    }
    let Some(run) = state.workflow_runs.get(&job.run_id) else {
        return String::new();
    };
    if run.kind != RunKind::QuickTask {
        return String::new();
    }
    let Some(gate) = run
        .gates
        .iter()
        .rev()
        .find(|gate| gate.state == crate::workflows::gates::HumanGateState::AwaitingDecision)
    else {
        return String::new();
    };
    format!("/runs/{}/gates/{}", run.id.as_hex(), gate.id.as_hex())
}

async fn attach_sandbox_status(state: &AppState, page: &mut ChatViewModel) {
    // This cursor precedes the status reads so concurrent catalogue changes remain observable.
    let cursor = state.environments.refresh_cursor();
    let seed_id = state
        .environments
        .seed_id(crate::environments::seeds::ALPINE_GIT_V1);
    let record = seed_id.and_then(|id| state.environments.get(&id));
    let latest = record
        .as_ref()
        .and_then(|record| state.environments.preparation(&record.latest_preparation));
    let ready_availability = match record.as_ref() {
        Some(record) => match state.environments.copy_ready_pointer(&record.id) {
            Ok(pointer) => Some(state.environment_snapshots.inspect(&pointer.snapshot).await),
            Err(_) => None,
        },
        None => None,
    };
    page.sandbox_status =
        page::SandboxStatus::from_parts(record.as_ref(), latest.as_ref(), ready_availability);
    page.quick_ready = page.sandbox_status.is_ready();
    page.sandbox_cursor = EnvironmentCatalogue::cursor_token(cursor);
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
        page.workflow_options = Vec::new();
        page.workflow_empty = true;
    } else {
        let mut preferred = session.preferred_workflow;
        if let Some(id) = preferred
            && state.workflows.get(&id).is_none()
        {
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
                policy: workflow_policy(&record.definition),
                selected: selected == Some(record.id),
            })
            .collect();
    }
}

fn workflow_policy(definition: &crate::workflows::definition::WorkflowDefinition) -> String {
    use crate::workflows::definition::{
        ArtefactKind, ArtefactSource, CandidateAuthority, StepAction,
    };

    let Some(commit) = definition.steps().iter().find(|step| {
        matches!(
            &step.action,
            StepAction::SystemCommand(action)
                if action.command == crate::workflows::commands::SystemCommandId::CommitCandidate
        )
    }) else {
        return "No review policy".to_owned();
    };
    let source_step = |kind| {
        commit
            .inputs
            .iter()
            .find(|input| input.kind == kind)
            .and_then(|input| match &input.source {
                ArtefactSource::StepOutput { step, .. } => definition.step(step),
                ArtefactSource::RunInitialCandidate | ArtefactSource::RunCurrentCandidate => None,
            })
    };
    let (Some(candidate_step), Some(report_step)) = (
        source_step(ArtefactKind::CandidateRevision),
        source_step(ArtefactKind::ReviewReport),
    ) else {
        return "No review policy".to_owned();
    };
    let StepAction::Agent(report_action) = &report_step.action else {
        return "No review policy".to_owned();
    };
    match report_action.candidate_authority {
        CandidateAuthority::Edit if candidate_step.key == report_step.key => {
            "Fixing review with direct commit policy".to_owned()
        }
        CandidateAuthority::ReadOnly => {
            let reviewed_candidate = report_step.inputs.iter().find_map(|input| {
                if input.kind != ArtefactKind::CandidateRevision {
                    return None;
                }
                match &input.source {
                    ArtefactSource::StepOutput { step, .. } => Some(step),
                    ArtefactSource::RunInitialCandidate | ArtefactSource::RunCurrentCandidate => {
                        None
                    }
                }
            });
            let fixing_candidate = reviewed_candidate
                .and_then(|step| definition.step(step))
                .is_some_and(|step| {
                    matches!(
                        &step.action,
                        StepAction::Agent(action)
                            if action.candidate_authority == CandidateAuthority::Edit
                                && action.required_outputs.iter().any(|output| {
                                    output.kind
                                        == crate::workflows::definition::OutputKind::ReviewReport
                                })
                    )
                });
            if candidate_step.key == report_step.key {
                "No review policy".to_owned()
            } else if fixing_candidate {
                "Fixing review with independent read-only review".to_owned()
            } else {
                "Read-only review before commit".to_owned()
            }
        }
        CandidateAuthority::Edit => "No review policy".to_owned(),
    }
}

async fn desk_view(state: &AppState, desk: &ResolvedDesk) -> ChatViewModel {
    view(state, desk.page(), "", "", "").await
}

fn resolved_desk(
    state: &AppState,
    session: SessionId,
    project: &str,
    agent: &str,
) -> Option<ResolvedDesk> {
    let project_id = ProjectId::parse(project.trim())?;
    let agent_id = crate::agents::AgentId::parse(agent.trim())?;
    let project = state.projects.get(&project_id)?;
    let record = state.agents.get(&agent_id)?;
    eligibility(&record, &project)?;
    let key = ConversationKey {
        project_id: project.id,
        agent_id: record.id,
    };
    let snapshot = state.sessions.snapshot(&session, &key)?;
    let eligible = eligible_agents(&state.agents.list(), &project);
    Some(ResolvedDesk {
        project,
        agent: record,
        eligible,
        snapshot,
    })
}

pub(crate) fn navigate_page(state: &AppState, view: &ChatViewModel) -> AppResult<Response> {
    match hypergraft::outcome::page_patch(&view.document_title, "chat-main", view) {
        Ok(response) => Ok(response),
        Err(error) if error.kind() == hypergraft::PatchBuildErrorKind::ResponseLimit => {
            crate::error::trace_patch_build_failure("construct chat page navigation patch", &error);
            responses::chat_page_response(&view.document_title, state, view)
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn render_document(
    state: &AppState,
    status: PatchStatus,
    view: ChatViewModel,
) -> AppResult<Response> {
    let mut response = responses::chat_page_response(&view.document_title, state, &view)?;
    responses::apply_patch_status(&mut response, status);
    Ok(response)
}
