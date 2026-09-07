mod page;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::Response,
    routing::{get, post},
};
use hypergraft::{GraftRequest, PageGraft, PatchGraft, PatchStatus};

use crate::{
    error::AppResult,
    responses,
    sessions::RequiredSession,
    state::AppState,
    workflows::{RunId, TaskLoop, TaskLoopId},
};

use self::page::{
    ArtefactView, LoopDetailView, RunDetailView, RunIndexView, attempt_activity_view,
    attempt_changes_view, attempt_result_view,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/runs", get(index))
        .route("/runs/loops/{loop_id}", get(loop_detail))
        .route("/runs/loops/{loop_id}/pause", post(pause_loop))
        .route("/runs/loops/{loop_id}/continue", post(continue_loop))
        .route("/runs/loops/{loop_id}/retry", post(retry_loop))
        .route("/runs/loops/{loop_id}/stop", post(stop_loop))
        .route("/runs/{run_id}", get(detail))
        .route(
            "/runs/{run_id}/attempts/{attempt_id}/context",
            get(initial_context),
        )
        .route(
            "/runs/{run_id}/attempts/{attempt_id}/activity",
            get(attempt_activity),
        )
        .route(
            "/runs/{run_id}/attempts/{attempt_id}/changes",
            get(attempt_changes),
        )
        .route(
            "/runs/{run_id}/attempts/{attempt_id}/result",
            get(attempt_result),
        )
        .route("/runs/{run_id}/artefacts/{artefact_id}", get(artefact))
}

async fn index(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
    let view = RunIndexView::combined(&state);
    match graft {
        PageGraft::Document => {
            let mut response = responses::chat_page_response(page::INDEX_TITLE, &state, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Ok);
            Ok(response)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            page::INDEX_TITLE,
            "chat-main",
            &view,
        )?),
    }
}

async fn loop_detail(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Path(loop_id): Path<String>,
) -> AppResult<Response> {
    let Some(id) = TaskLoopId::parse(&loop_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(record) = state.task_loops.get(&id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let view = LoopDetailView::from_loop(&record, loop_awaiting_gate(&state, &record));
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(page::DETAIL_TITLE, &state, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Ok);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            page::DETAIL_TITLE,
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(responses::request_navigation(
            graft,
            &format!("/runs/loops/{}", id.as_hex()),
        )),
    }
}

async fn detail(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Path(run_id): Path<String>,
) -> AppResult<Response> {
    let Some(id) = RunId::parse(&run_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(run) = state.workflow_runs.get(&id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let parent = run.parent_loop.and_then(|id| state.task_loops.get(&id));
    let view = RunDetailView::from_run(
        &run,
        &state.workflows,
        &state.environments,
        &state.projects,
        &state.workflow_evidence,
        parent.as_ref(),
    );
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(page::DETAIL_TITLE, &state, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Ok);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            page::DETAIL_TITLE,
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "run-detail",
            &view.contents(),
        )?),
    }
}

#[derive(Default, serde::Deserialize)]
struct ContextQuery {
    #[serde(default)]
    part: usize,
    #[serde(default)]
    offset: usize,
}

async fn initial_context(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path((run_id, attempt_id)): Path<(String, String)>,
    Query(query): Query<ContextQuery>,
) -> AppResult<Response> {
    let Some(run) = RunId::parse(&run_id).and_then(|id| state.workflow_runs.get(&id)) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let run_href = format!("/runs/{}", run.id.as_hex());
    let Some(attempt) = run
        .attempts
        .iter()
        .find(|attempt| attempt.id.as_hex() == attempt_id)
    else {
        return Ok(responses::request_navigation(graft, &run_href));
    };
    let context_href = format!("{run_href}/attempts/{}/context", attempt.id.as_hex());
    let Some(view) = attempt.initial_context.as_ref().and_then(|packet| {
        page::initial_context_view(packet, &run_href, &context_href, query.part, query.offset)
    }) else {
        return Ok(responses::request_navigation(graft, &run_href));
    };
    match graft {
        PageGraft::Document => responses::chat_page_response("Initial context", &state, &view),
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            "Initial context",
            "chat-main",
            &view,
        )?),
    }
}

async fn attempt_activity(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path((run_id, attempt_id)): Path<(String, String)>,
) -> AppResult<Response> {
    let Some(run_id) = RunId::parse(&run_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(run) = state.workflow_runs.get(&run_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(attempt) = run
        .attempts
        .iter()
        .find(|attempt| attempt.id.as_hex() == attempt_id)
    else {
        return Ok(responses::request_navigation(
            graft,
            &format!("/runs/{}", run.id.as_hex()),
        ));
    };
    let evidence = crate::workflows::AttemptId::parse(&attempt_id)
        .and_then(|attempt_id| state.workflow_evidence.get(&run.id, &attempt_id));
    let view = attempt_activity_view(&run, attempt, evidence);
    match graft {
        PageGraft::Document => responses::chat_page_response("Attempt activity", &state, &view),
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            "Attempt activity",
            "chat-main",
            &view,
        )?),
    }
}

async fn attempt_changes(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path((run_id, attempt_id)): Path<(String, String)>,
) -> AppResult<Response> {
    let Some(run_id) = RunId::parse(&run_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(run) = state.workflow_runs.get(&run_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(attempt) = run
        .attempts
        .iter()
        .find(|attempt| attempt.id.as_hex() == attempt_id)
    else {
        return Ok(responses::request_navigation(
            graft,
            &format!("/runs/{}", run.id.as_hex()),
        ));
    };
    let view = attempt_changes_view(&run, attempt, &state);
    match graft {
        PageGraft::Document => responses::chat_page_response("Attempt changes", &state, &view),
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            "Attempt changes",
            "chat-main",
            &view,
        )?),
    }
}

async fn attempt_result(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path((run_id, attempt_id)): Path<(String, String)>,
) -> AppResult<Response> {
    let Some(run_id) = RunId::parse(&run_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(run) = state.workflow_runs.get(&run_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(attempt) = run
        .attempts
        .iter()
        .find(|attempt| attempt.id.as_hex() == attempt_id)
    else {
        return Ok(responses::request_navigation(
            graft,
            &format!("/runs/{}", run.id.as_hex()),
        ));
    };
    let evidence = crate::workflows::AttemptId::parse(&attempt_id)
        .and_then(|attempt_id| state.workflow_evidence.get(&run.id, &attempt_id));
    let view = attempt_result_view(&run, attempt, evidence);
    match graft {
        PageGraft::Document => responses::chat_page_response("Attempt result", &state, &view),
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            "Attempt result",
            "chat-main",
            &view,
        )?),
    }
}

async fn artefact(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path((run_id, artefact_id)): Path<(String, String)>,
) -> AppResult<Response> {
    let Some(run_id) = RunId::parse(&run_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(artefact_id) = crate::workflows::ArtefactId::parse(&artefact_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(run) = state.workflow_runs.get(&run_id) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    let Some(record) = run.artefact(&artefact_id) else {
        return Ok(responses::request_navigation(
            graft,
            &format!("/runs/{}", run.id.as_hex()),
        ));
    };
    let view = ArtefactView::from_record(&run, record, &state);
    match graft {
        PageGraft::Document => {
            let mut response = responses::chat_page_response(page::ARTEFACT_TITLE, &state, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Ok);
            Ok(response)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            page::ARTEFACT_TITLE,
            "chat-main",
            &view,
        )?),
    }
}

#[derive(Default, serde::Deserialize)]
struct LoopCommandForm {
    #[serde(default)]
    token: String,
    #[serde(default)]
    surface: String,
}

fn loop_awaiting_gate(state: &AppState, record: &TaskLoop) -> bool {
    record.current_child().is_some_and(|child| {
        state.workflow_runs.get(&child).is_some_and(|run| {
            run.gates
                .iter()
                .any(|gate| gate.state == crate::workflows::gates::HumanGateState::AwaitingDecision)
        })
    })
}

fn conversation_surface(form: &LoopCommandForm) -> bool {
    form.surface == "conversation"
}

fn command_success(
    state: &AppState,
    session: crate::sessions::SessionId,
    record: &TaskLoop,
    form: &LoopCommandForm,
) -> AppResult<Response> {
    if conversation_surface(form) {
        super::conversations::refresh_after_loop_command(state, session, &record.conversation_id)
    } else {
        Ok(responses::command_navigation(&format!(
            "/runs/loops/{}",
            record.id.as_hex()
        )))
    }
}

fn controls_patch(
    record: &TaskLoop,
    awaiting_gate: bool,
    error: &'static str,
    conversation: bool,
    status: PatchStatus,
) -> AppResult<Response> {
    let view = page::loop_controls(record, awaiting_gate, error, conversation);
    Ok(hypergraft::outcome::children_patch(
        status,
        "loop-controls",
        &view,
    )?)
}

fn command_error(
    record: &TaskLoop,
    state: &AppState,
    form: &LoopCommandForm,
    status: PatchStatus,
    message: &'static str,
) -> AppResult<Response> {
    controls_patch(
        record,
        loop_awaiting_gate(state, record),
        message,
        conversation_surface(form),
        status,
    )
}

fn parse_loop(raw: &str, state: &AppState) -> Option<TaskLoop> {
    TaskLoopId::parse(raw).and_then(|id| state.task_loops.get(&id))
}

async fn pause_loop(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(loop_id): Path<String>,
    Form(form): Form<LoopCommandForm>,
) -> AppResult<Response> {
    let Some(record) = parse_loop(&loop_id, &state) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    match state.task_loops.request_pause(&record.id, &form.token) {
        Ok(updated) => controls_patch(
            &updated,
            loop_awaiting_gate(&state, &updated),
            "",
            conversation_surface(&form),
            PatchStatus::Ok,
        ),
        Err(error) => command_error(
            &record,
            &state,
            &form,
            match error {
                crate::workflows::TaskLoopError::Stale => PatchStatus::Conflict,
                _ => PatchStatus::UnprocessableEntity,
            },
            error.message(),
        ),
    }
}

async fn continue_loop(
    State(state): State<AppState>,
    RequiredSession(session): RequiredSession,
    graft: PatchGraft,
    Path(loop_id): Path<String>,
    Form(form): Form<LoopCommandForm>,
) -> AppResult<Response> {
    let Some(record) = parse_loop(&loop_id, &state) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    if record.command_token() != form.token {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Stale.message(),
        );
    }
    if !record.allows_continue() {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Conflict.message(),
        );
    }
    if loop_has_uncertain_commit(&state, &record) {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Uncertain.message(),
        );
    }
    let mut parked = state.gate_continuations.take_paused(&record.id);
    let (job, source) = match bind_loop_continuation(&state, session, &record, parked.as_mut()) {
        Ok(bound) => bound,
        Err(message) => {
            if let Some(checkpoint) = parked {
                state
                    .gate_continuations
                    .put_back_paused(record.id, checkpoint);
            }
            return command_error(&record, &state, &form, PatchStatus::Conflict, message);
        }
    };
    let Ok(execution) = state.workflow_execution.acquire() else {
        release_loop_continuation(&state, session, &job, parked);
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Busy.message(),
        );
    };
    if let Err(message) = revalidate_paused_loop(&state, &record, &job, &source) {
        drop(execution);
        release_loop_continuation(&state, session, &job, parked);
        return command_error(&record, &state, &form, PatchStatus::Conflict, message);
    }
    let aggregate = match task_loop_attempt_total(&state, &record) {
        Ok(total) => total,
        Err(message) => {
            drop(execution);
            release_loop_continuation(&state, session, &job, parked);
            return command_error(&record, &state, &form, PatchStatus::Conflict, message);
        }
    };
    let reserved = match state.task_loops.reserve_next_child(&record.id, aggregate) {
        Ok(reserved) => reserved,
        Err(error) => {
            drop(execution);
            let current = state.task_loops.get(&record.id).unwrap_or(record.clone());
            if current.state.is_terminal() {
                crate::workflows::settle_cancelled_job(&state, &job);
                return command_success(&state, session, &current, &form);
            }
            release_loop_continuation(&state, session, &job, parked);
            return command_error(
                &current,
                &state,
                &form,
                PatchStatus::Conflict,
                error.message(),
            );
        }
    };
    dispatch_loop_child(&state, session, &form, execution, job, parked, reserved)
}

async fn retry_loop(
    State(state): State<AppState>,
    RequiredSession(session): RequiredSession,
    graft: PatchGraft,
    Path(loop_id): Path<String>,
    Form(form): Form<LoopCommandForm>,
) -> AppResult<Response> {
    let Some(record) = parse_loop(&loop_id, &state) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    if record.command_token() != form.token {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Stale.message(),
        );
    }
    if !record.allows_retry() {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Conflict.message(),
        );
    }
    if loop_has_uncertain_commit(&state, &record) {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Uncertain.message(),
        );
    }
    let task = record.retryable_task().cloned();
    let Some(task) = task else {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Conflict.message(),
        );
    };
    let child_id = task.child_id.expect("retryable child");
    if state
        .workflow_runs
        .get(&child_id)
        .is_some_and(|run| crate::workflows::task_loop::child_commit_uncertain(&run))
    {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Uncertain.message(),
        );
    }
    let reuse_reserved = task.outcome == crate::workflows::TaskOutcome::Reserved
        && state.workflow_runs.get(&child_id).is_none();
    let (job, source) = match bind_loop_continuation(&state, session, &record, None) {
        Ok(bound) => bound,
        Err(message) => {
            return command_error(&record, &state, &form, PatchStatus::Conflict, message);
        }
    };
    let Ok(execution) = state.workflow_execution.acquire() else {
        release_loop_continuation(&state, session, &job, None);
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Busy.message(),
        );
    };
    if let Err(message) = revalidate_retry_loop(&state, &record, &job, child_id, &source) {
        drop(execution);
        release_loop_continuation(&state, session, &job, None);
        return command_error(&record, &state, &form, PatchStatus::Conflict, message);
    }
    let aggregate = match task_loop_attempt_total(&state, &record) {
        Ok(total) => total,
        Err(message) => {
            drop(execution);
            release_loop_continuation(&state, session, &job, None);
            return command_error(&record, &state, &form, PatchStatus::Conflict, message);
        }
    };
    let reserved = match state
        .task_loops
        .retry_current(&record.id, aggregate, reuse_reserved)
    {
        Ok(reserved) => reserved,
        Err(error) => {
            drop(execution);
            let current = state.task_loops.get(&record.id).unwrap_or(record.clone());
            if current.state.is_terminal() {
                crate::workflows::settle_cancelled_job(&state, &job);
                return command_success(&state, session, &current, &form);
            }
            release_loop_continuation(&state, session, &job, None);
            return command_error(
                &current,
                &state,
                &form,
                PatchStatus::Conflict,
                error.message(),
            );
        }
    };
    dispatch_loop_child(&state, session, &form, execution, job, None, reserved)
}

async fn stop_loop(
    State(state): State<AppState>,
    RequiredSession(session): RequiredSession,
    graft: PatchGraft,
    Path(loop_id): Path<String>,
    Form(form): Form<LoopCommandForm>,
) -> AppResult<Response> {
    let Some(record) = parse_loop(&loop_id, &state) else {
        return Ok(responses::request_navigation(graft, "/runs"));
    };
    if record.command_token() != form.token {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Stale.message(),
        );
    }
    if record.state.is_terminal() {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Conflict.message(),
        );
    }
    if record.allows_continue() || record.allows_retry() {
        if let Some(job) = state.gate_continuations.take_paused(&record.id) {
            match state.task_loops.stop_if_token(&record.id, &form.token) {
                Ok(_) => {
                    crate::workflows::settle_cancelled_job(&state, &job.job);
                    return command_success(&state, session, &record, &form);
                }
                Err(error) => {
                    state.gate_continuations.put_back_paused(record.id, job);
                    return command_error(
                        &record,
                        &state,
                        &form,
                        PatchStatus::Conflict,
                        error.message(),
                    );
                }
            }
        }
        match state.task_loops.stop_if_token(&record.id, &form.token) {
            Ok(_) => {
                if let Some(request) = state
                    .conversations
                    .get(&record.conversation_id)
                    .and_then(|conversation| conversation.active_job)
                {
                    let _ = state.conversations.settle_message(
                        &record.conversation_id,
                        request,
                        String::new(),
                        crate::conversations::MessageStatus::Interrupted,
                    );
                }
                return command_success(&state, session, &record, &form);
            }
            Err(error) => {
                return command_error(
                    &record,
                    &state,
                    &form,
                    PatchStatus::Conflict,
                    error.message(),
                );
            }
        }
    }
    if let Some(child) = record.current_child()
        && state.gate_continuations.available(&child, &session)
    {
        let Some(continuation) = state.gate_continuations.take(&child) else {
            return command_error(
                &record,
                &state,
                &form,
                PatchStatus::Conflict,
                crate::workflows::TaskLoopError::Conflict.message(),
            );
        };
        match state
            .task_loops
            .stop_at_gate(&record.id, &form.token, &state.workflow_runs)
        {
            Ok(_) => {
                crate::workflows::settle_cancelled_job(&state, &continuation);
                return command_success(&state, session, &record, &form);
            }
            Err(error) => {
                state.gate_continuations.put_back(continuation);
                return command_error(
                    &record,
                    &state,
                    &form,
                    PatchStatus::Conflict,
                    error.message(),
                );
            }
        }
    }
    let Some(job) = state
        .conversations
        .get(&record.conversation_id)
        .and_then(|conversation| conversation.active_job)
        .and_then(|job_id| {
            state
                .sessions
                .conversation_job(record.conversation_id, job_id)
        })
    else {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Conflict.message(),
        );
    };
    if job.snapshot().status == crate::sessions::JobStatus::AwaitingDecision {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            "This decision belongs to another browser session.",
        );
    }
    if let Err(error) = state.task_loops.request_stop(&record.id, &form.token, &job) {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            error.message(),
        );
    }
    controls_patch(
        &record,
        loop_awaiting_gate(&state, &record),
        "",
        conversation_surface(&form),
        PatchStatus::Ok,
    )
}

fn loop_has_uncertain_commit(state: &AppState, record: &TaskLoop) -> bool {
    if state.gate_continuations.commit_recovery_locked() {
        return true;
    }
    record.tasks.iter().any(|task| {
        task.child_id
            .into_iter()
            .chain(task.previous_child_ids.iter().copied())
            .any(|id| {
                state
                    .workflow_runs
                    .get(&id)
                    .is_some_and(|run| crate::workflows::task_loop::child_commit_uncertain(&run))
            })
    })
}

fn task_loop_attempt_total(state: &AppState, parent: &TaskLoop) -> Result<usize, &'static str> {
    parent.tasks.iter().try_fold(0usize, |total, task| {
        task.child_id
            .into_iter()
            .chain(task.previous_child_ids.iter().copied())
            .try_fold(total, |total, id| {
                let child = state.workflow_runs.get(&id);
                let attempts = child.map(|run| run.attempts.len()).unwrap_or(0);
                total
                    .checked_add(attempts)
                    .ok_or("The task loop reached its attempt bound.")
            })
    })
}

fn revalidate_paused_loop(
    state: &AppState,
    record: &TaskLoop,
    job: &crate::workflows::WorkflowJob,
    source: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
) -> Result<(), &'static str> {
    let Some(project) = state.projects.get(&record.project_id) else {
        return Err("The target project is no longer available.");
    };
    if !project.host_path_is_available() {
        return Err("A granted directory is no longer at the saved path.");
    }
    if let Some(conversation_id) = job.conversation_id {
        let Some(pinned) = job.authority.as_ref() else {
            return Err("The conversation authority does not match this run.");
        };
        let current = match state.conversations.get(&conversation_id) {
            Some(conversation) => match crate::conversations::resolve_workflow_authority(
                &conversation,
                &state.projects,
                &state.agents,
            ) {
                Ok(Some(authority)) => authority.effective,
                Ok(None) | Err(_) => {
                    return Err("The conversation authority does not match this run.");
                }
            },
            None => return Err("The conversation authority does not match this run."),
        };
        if current != *pinned || pinned.revalidate_project(&project).is_err() {
            return Err("The conversation authority does not match this run.");
        }
    }
    for phase in &record.phase_models {
        crate::workflows::validate_phase_selection(state, &phase.selection)
            .map_err(|_| "A selected phase provider or model is no longer available.")?;
    }
    if record
        .phase_models
        .iter()
        .filter_map(|phase| phase.preset.as_ref())
        .any(|preset| {
            state
                .agents
                .get(&preset.id)
                .is_none_or(|agent| agent.revision != preset.revision)
        })
    {
        return Err("A pinned preset is no longer available.");
    }
    if !crate::workflows::artefacts::CandidateCapture::capture_host(
        &project.host_path,
        &state.workflow_artefacts,
    )
    .is_ok_and(|current| current == *source)
    {
        return Err("The project source has changed since the last completed task.");
    }
    Ok(())
}

fn bind_loop_continuation(
    state: &AppState,
    session: crate::sessions::SessionId,
    record: &TaskLoop,
    parked: Option<&mut crate::workflows::PausedWorkflow>,
) -> Result<
    (
        crate::workflows::WorkflowJob,
        crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
    ),
    &'static str,
> {
    if let Some(checkpoint) = parked {
        if state
            .sessions
            .acquire_job_reservation(
                &session,
                checkpoint.job.conversation_id,
                checkpoint.job.job.id(),
            )
            .is_err()
        {
            return Err(crate::workflows::TaskLoopError::Busy.message());
        }
        checkpoint.job.session_id = session;
        return Ok((checkpoint.job.clone(), checkpoint.source.clone()));
    }
    let conversation = state
        .conversations
        .get(&record.conversation_id)
        .ok_or("The conversation for this task loop is no longer available.")?;
    let session_job = if let Some(job_id) = conversation.active_job {
        let assistant_index = conversation
            .messages
            .iter()
            .rposition(|message| message.request == Some(job_id))
            .unwrap_or(conversation.messages.len());
        state
            .sessions
            .attach_conversation_job(&session, record.conversation_id, job_id, assistant_index)
            .map_err(|_| crate::workflows::TaskLoopError::Busy.message())?
    } else {
        let job = state
            .sessions
            .begin_conversation_job(
                &session,
                record.conversation_id,
                conversation.messages.len(),
            )
            .map_err(|_| crate::workflows::TaskLoopError::Busy.message())?;
        if state
            .conversations
            .reopen_loop_request(&record.conversation_id, job.id())
            .is_err()
        {
            state
                .sessions
                .finish_conversation_job(&session, record.conversation_id, job.id());
            return Err("Power Plant could not reserve this conversation.");
        }
        job
    };
    let placeholder = record
        .occupied_child()
        .or_else(|| {
            record.tasks.iter().rev().find_map(|task| {
                task.child_id
                    .or_else(|| task.previous_child_ids.last().copied())
            })
        })
        .or_else(|| RunId::parse(&"0".repeat(32)))
        .expect("placeholder");
    let job = match crate::workflows::reconstruct_loop_job(
        state,
        session,
        session_job.clone(),
        record,
        placeholder,
    ) {
        Ok(job) => job,
        Err(message) => {
            let _ = state.sessions.release_job_reservation(
                &session,
                Some(record.conversation_id),
                session_job.id(),
            );
            return Err(message);
        }
    };
    let source = match loop_checkpoint_source(state, record) {
        Ok(source) => source,
        Err(message) => {
            let _ = state.sessions.release_job_reservation(
                &session,
                Some(record.conversation_id),
                session_job.id(),
            );
            return Err(message);
        }
    };
    Ok((job, source))
}

fn release_loop_continuation(
    state: &AppState,
    session: crate::sessions::SessionId,
    job: &crate::workflows::WorkflowJob,
    parked: Option<crate::workflows::PausedWorkflow>,
) {
    let _ = state
        .sessions
        .release_job_reservation(&session, job.conversation_id, job.job.id());
    if let Some(checkpoint) = parked {
        let loop_id = checkpoint.job.task_loop.or(job.task_loop).expect("loop");
        state
            .gate_continuations
            .put_back_paused(loop_id, checkpoint);
    }
}

fn dispatch_loop_child(
    state: &AppState,
    session: crate::sessions::SessionId,
    form: &LoopCommandForm,
    execution: crate::workflows::ExecutionGuard,
    mut job: crate::workflows::WorkflowJob,
    parked: Option<crate::workflows::PausedWorkflow>,
    reserved: (
        TaskLoop,
        crate::workflows::RunId,
        crate::workflows::TaskLoopItem,
    ),
) -> AppResult<Response> {
    let (parent, child_id, task) = reserved;
    let run = match parent.child_run(
        child_id,
        crate::workflows::now_ms(),
        task.index,
        task.markdown,
    ) {
        Ok(run) => run,
        Err(error) => {
            drop(execution);
            release_loop_continuation(state, session, &job, parked);
            let _ = state.task_loops.fail(&parent.id);
            job.job.set_awaiting_decision();
            return command_error(&parent, state, form, PatchStatus::Conflict, error.message());
        }
    };
    if state.workflow_runs.create(run).is_err()
        || state
            .task_loops
            .mark_dispatched(&parent.id, child_id)
            .is_err()
    {
        drop(execution);
        release_loop_continuation(state, session, &job, parked);
        let _ = state.task_loops.fail(&parent.id);
        job.job.set_awaiting_decision();
        return command_error(
            &parent,
            state,
            form,
            PatchStatus::Conflict,
            "Power Plant could not start the next task.",
        );
    }
    job.run_id = child_id;
    job.session_id = session;
    job.eligible_reply
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    let _ = job.job.resume();
    job.job.set_step_label("Source capture".to_owned());
    tokio::spawn(crate::workflows::execute_run(
        state.clone(),
        job,
        None,
        execution,
    ));
    command_success(state, session, &parent, form)
}

fn loop_checkpoint_source(
    state: &AppState,
    record: &TaskLoop,
) -> Result<crate::workflows::artefacts::candidate::CandidateRevisionArtefact, &'static str> {
    let checkpoint = record
        .retryable_task()
        .and_then(|task| task.child_id)
        .and_then(|id| state.workflow_runs.get(&id))
        .and_then(|run| match &run.source {
            crate::workflows::RunSource::Captured { source } => {
                Some((run.clone(), source.initial.clone()))
            }
            _ => None,
        })
        .or_else(|| {
            record.tasks.iter().rev().find_map(|task| {
                if !matches!(
                    task.outcome,
                    crate::workflows::TaskOutcome::CompletedCommit
                        | crate::workflows::TaskOutcome::CompletedUnchanged
                ) {
                    return None;
                }
                let run = state.workflow_runs.get(&task.child_id?)?;
                match &run.source {
                    crate::workflows::RunSource::Captured { source } => {
                        Some((run.clone(), source.accepted.clone()))
                    }
                    _ => None,
                }
            })
        });
    if let Some((run, reference)) = checkpoint {
        let artefact = run
            .artefact(&reference.id)
            .filter(|artefact| artefact.artefact_hash == reference.artefact_hash)
            .ok_or("The recorded checkpoint is no longer available.")?;
        let bytes = state
            .workflow_artefacts
            .get(&artefact.object_hash)
            .map_err(|_| "The recorded checkpoint is no longer available.")?;
        return crate::workflows::artefacts::candidate::CandidateRevisionArtefact::from_manifest_bytes(&bytes)
            .ok_or("The recorded checkpoint is no longer available.");
    }
    if record.completed_count() > 0 || record.allows_retry() {
        return Err(
            "No durable task base exists. This task remains available for inspection only.",
        );
    }
    let project = state
        .projects
        .get(&record.project_id)
        .ok_or("The target project is no longer available.")?;
    crate::workflows::artefacts::CandidateCapture::capture_host(
        &project.host_path,
        &state.workflow_artefacts,
    )
    .map_err(|_| "Power Plant could not capture the project source.")
}

fn revalidate_retry_loop(
    state: &AppState,
    record: &TaskLoop,
    job: &crate::workflows::WorkflowJob,
    previous_child: crate::workflows::RunId,
    source: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
) -> Result<(), &'static str> {
    revalidate_paused_loop(state, record, job, source)?;
    let Some(previous) = state.workflow_runs.get(&previous_child) else {
        return Ok(());
    };
    let crate::workflows::RunSource::Captured {
        source: ref captured,
    } = previous.source
    else {
        return Ok(());
    };
    let Some(base) = previous.artefact(&captured.initial.id).cloned() else {
        return Err("The recorded task base is no longer available.");
    };
    if base.artefact_hash != captured.initial.artefact_hash {
        return Err("The recorded task base is no longer available.");
    }
    let bytes = state
        .workflow_artefacts
        .get(&base.object_hash)
        .map_err(|_| "The recorded task base is no longer available.")?;
    let initial =
        crate::workflows::artefacts::candidate::CandidateRevisionArtefact::from_manifest_bytes(
            &bytes,
        )
        .ok_or("The recorded task base is no longer available.")?;
    if *source != initial {
        return Err("The project source has changed since this task started.");
    }
    Ok(())
}
