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
    let view = RunDetailView::from_run(
        &run,
        &state.workflows,
        &state.environments,
        &state.projects,
        &state.workflow_evidence,
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
    if !matches!(
        record.state,
        crate::workflows::task_loop::TaskLoopState::Paused
    ) {
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
    let Some(mut checkpoint) = state.gate_continuations.take_paused(&record.id) else {
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Conflict.message(),
        );
    };
    let job = &mut checkpoint.job;
    if state
        .sessions
        .acquire_job_reservation(&session, job.conversation_id, job.job.id())
        .is_err()
    {
        state
            .gate_continuations
            .put_back_paused(record.id, checkpoint);
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Busy.message(),
        );
    }
    job.session_id = session;
    let Ok(execution) = state.workflow_execution.acquire() else {
        let _ = state
            .sessions
            .release_job_reservation(&session, job.conversation_id, job.job.id());
        state
            .gate_continuations
            .put_back_paused(record.id, checkpoint);
        return command_error(
            &record,
            &state,
            &form,
            PatchStatus::Conflict,
            crate::workflows::TaskLoopError::Busy.message(),
        );
    };
    if let Err(message) = revalidate_paused_loop(&state, &record, job, &checkpoint.source) {
        drop(execution);
        let _ = state
            .sessions
            .release_job_reservation(&session, job.conversation_id, job.job.id());
        state
            .gate_continuations
            .put_back_paused(record.id, checkpoint);
        return command_error(&record, &state, &form, PatchStatus::Conflict, message);
    }
    let aggregate = match task_loop_attempt_total(&state, &record) {
        Ok(total) => total,
        Err(message) => {
            drop(execution);
            let _ =
                state
                    .sessions
                    .release_job_reservation(&session, job.conversation_id, job.job.id());
            state
                .gate_continuations
                .put_back_paused(record.id, checkpoint);
            return command_error(&record, &state, &form, PatchStatus::Conflict, message);
        }
    };
    let reserved = match state.task_loops.reserve_next_child(&record.id, aggregate) {
        Ok(reserved) => reserved,
        Err(error) => {
            drop(execution);
            let current = state.task_loops.get(&record.id).unwrap_or(record.clone());
            if current.state.is_terminal() {
                crate::workflows::settle_cancelled_job(&state, job);
                return command_success(&state, session, &current, &form);
            }
            let _ =
                state
                    .sessions
                    .release_job_reservation(&session, job.conversation_id, job.job.id());
            state
                .gate_continuations
                .put_back_paused(record.id, checkpoint);
            return command_error(
                &current,
                &state,
                &form,
                PatchStatus::Conflict,
                error.message(),
            );
        }
    };
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
            let _ =
                state
                    .sessions
                    .release_job_reservation(&session, job.conversation_id, job.job.id());
            let _ = state.task_loops.fail(&parent.id);
            crate::workflows::settle_cancelled_job(&state, job);
            return command_error(
                &parent,
                &state,
                &form,
                PatchStatus::Conflict,
                error.message(),
            );
        }
    };
    if state.workflow_runs.create(run).is_err()
        || state
            .task_loops
            .mark_dispatched(&parent.id, child_id)
            .is_err()
    {
        drop(execution);
        let _ = state
            .sessions
            .release_job_reservation(&session, job.conversation_id, job.job.id());
        let _ = state.task_loops.fail(&parent.id);
        crate::workflows::settle_cancelled_job(&state, job);
        return command_error(
            &parent,
            &state,
            &form,
            PatchStatus::Conflict,
            "Power Plant could not start the next task.",
        );
    }
    let mut job = checkpoint.job;
    job.run_id = child_id;
    job.session_id = session;
    job.eligible_reply
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    job.job.resume();
    job.job.set_step_label("Source capture".to_owned());
    tokio::spawn(crate::workflows::execute_run(
        state.clone(),
        job,
        None,
        execution,
    ));
    command_success(&state, session, &parent, &form)
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
    if matches!(
        record.state,
        crate::workflows::task_loop::TaskLoopState::Paused
    ) {
        let Some(job) = state.gate_continuations.take_paused(&record.id) else {
            return command_error(
                &record,
                &state,
                &form,
                PatchStatus::Conflict,
                crate::workflows::TaskLoopError::Conflict.message(),
            );
        };
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
    record
        .tasks
        .iter()
        .filter_map(|task| task.child_id)
        .any(|id| {
            state.workflow_runs.get(&id).is_some_and(|run| {
                run.attempts.iter().any(|attempt| {
                    attempt.commit_transaction.is_some() && attempt.commit_result.is_none()
                })
            })
        })
}

fn task_loop_attempt_total(state: &AppState, parent: &TaskLoop) -> Result<usize, &'static str> {
    parent
        .tasks
        .iter()
        .filter_map(|task| task.child_id)
        .try_fold(0usize, |total, id| {
            let child = state
                .workflow_runs
                .get(&id)
                .ok_or("Power Plant could not read a child run.")?;
            total
                .checked_add(child.attempts.len())
                .ok_or("The task loop reached its attempt bound.")
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
