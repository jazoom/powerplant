mod forms;
mod page;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{get, post},
};
use hypergraft::{CommandGraft, PageGraft, PatchStatus};

use crate::{
    error::AppResult,
    responses,
    sessions::{JobStatus, RequiredSession, SessionId},
    state::AppState,
    workflows::{GateId, RunId},
};

#[derive(serde::Deserialize)]
struct RawQuery {
    page: Option<String>,
    change: Option<String>,
    line: Option<String>,
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/runs/{run_id}/gates/{gate_id}", get(detail))
        .route("/runs/{run_id}/gates/{gate_id}/approve", post(approve))
        .route(
            "/runs/{run_id}/gates/{gate_id}/request-revision",
            post(request_revision),
        )
        .route("/runs/{run_id}/gates/{gate_id}/cancel", post(cancel))
        .route(
            "/runs/{run_id}/gates/{gate_id}/objects/{side}/{change}",
            get(object),
        )
        .layer(axum::extract::DefaultBodyLimit::max(70 * 1024))
}

fn ids(run: &str, gate: &str) -> Option<(RunId, GateId)> {
    Some((RunId::parse(run)?, GateId::parse(gate)?))
}

async fn detail(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
    Path((run_id, gate_id)): Path<(String, String)>,
    Query(raw): Query<RawQuery>,
) -> AppResult<Response> {
    let Some((run_id, gate_id)) = ids(&run_id, &gate_id) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let Some(run) = state.workflow_runs.get(&run_id) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let Some(gate) = run.gates.iter().find(|gate| gate.id == gate_id) else {
        return Ok(responses::graft_redirect(
            graft,
            &format!("/runs/{}", run.id.as_hex()),
        ));
    };
    let Some(query) = forms::DiffQuery::parse(
        raw.page.as_deref(),
        raw.change.as_deref(),
        raw.line.as_deref(),
    ) else {
        return static_error(
            PatchStatus::UnprocessableEntity,
            graft,
            &state,
            "That diff page is not valid.",
        );
    };
    let Ok(diff) = crate::workflows::artefacts::CandidateDiff::load(
        &run,
        &gate.diff_base,
        &gate.candidate,
        &state.workflow_artefacts,
    ) else {
        return static_error(
            PatchStatus::UnprocessableEntity,
            graft,
            &state,
            "The immutable candidate diff is unavailable.",
        );
    };
    let Some(view) = page::GatePage::new(&run, gate, diff, &state.workflow_artefacts, query, "")
    else {
        return static_error(
            PatchStatus::UnprocessableEntity,
            graft,
            &state,
            "That diff page is not valid.",
        );
    };
    match graft {
        PageGraft::Document => {
            let mut response = responses::chat_page_response(page::TITLE, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Ok);
            Ok(response)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            page::TITLE,
            "chat-main",
            &view,
        )?),
    }
}

fn static_error(
    status: PatchStatus,
    graft: PageGraft,
    state: &AppState,
    message: &'static str,
) -> AppResult<Response> {
    #[derive(askama::Template)]
    #[template(
        source = "<main data-section=\"runs\" class=\"mx-auto max-w-4xl p-8\"><div role=\"alert\" class=\"alert alert-error\">{{ message }}</div><a href=\"/runs\" data-graft class=\"btn btn-ghost mt-4\">Runs</a></main>",
        ext = "html"
    )]
    struct ErrorView {
        message: &'static str,
    }
    let view = ErrorView { message };
    match graft {
        PageGraft::Document => {
            let mut response = responses::chat_page_response(page::TITLE, &state.assets, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            page::TITLE,
            "chat-main",
            &view,
        )?),
    }
}

async fn approve(
    State(state): State<AppState>,
    RequiredSession(session): RequiredSession,
    graft: CommandGraft,
    Path((run_id, gate_id)): Path<(String, String)>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    decide(
        state,
        session,
        graft,
        run_id,
        gate_id,
        pairs,
        DecisionAction::Approve,
    )
    .await
}

async fn request_revision(
    State(state): State<AppState>,
    RequiredSession(session): RequiredSession,
    graft: CommandGraft,
    Path((run_id, gate_id)): Path<(String, String)>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    decide(
        state,
        session,
        graft,
        run_id,
        gate_id,
        pairs,
        DecisionAction::Revision,
    )
    .await
}

async fn cancel(
    State(state): State<AppState>,
    RequiredSession(session): RequiredSession,
    graft: CommandGraft,
    Path((run_id, gate_id)): Path<(String, String)>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    decide(
        state,
        session,
        graft,
        run_id,
        gate_id,
        pairs,
        DecisionAction::Cancel,
    )
    .await
}

#[derive(Clone, Copy)]
enum DecisionAction {
    Approve,
    Revision,
    Cancel,
}

async fn decide(
    state: AppState,
    session: SessionId,
    graft: CommandGraft,
    run_raw: String,
    gate_raw: String,
    pairs: Vec<(String, String)>,
    action: DecisionAction,
) -> AppResult<Response> {
    let Some((run_id, gate_id)) = ids(&run_raw, &gate_raw) else {
        return Ok(responses::graft_redirect(graft, "/runs"));
    };
    let form = match forms::DecisionForm::parse(pairs, matches!(action, DecisionAction::Revision)) {
        Ok(form) => form,
        Err(forms::FormError::Note) => {
            return command_error(
                graft,
                PatchStatus::UnprocessableEntity,
                "Enter a revision note.",
            );
        }
        Err(forms::FormError::Invalid) => {
            return command_error(
                graft,
                PatchStatus::Conflict,
                "That gate page is stale. Reload it.",
            );
        }
    };
    let Some(run) = state.workflow_runs.get(&run_id) else {
        return command_error(graft, PatchStatus::Conflict, "That gate is unavailable.");
    };
    let Some(gate) = run.gates.iter().find(|gate| gate.id == gate_id) else {
        return command_error(graft, PatchStatus::Conflict, "That gate is unavailable.");
    };
    let target = run
        .artefact(&gate.candidate.id)
        .and_then(crate::workflows::artefacts::ArtefactRecord::candidate_hash)
        .map(|hash| hash.as_str());
    if gate.state != crate::workflows::gates::HumanGateState::AwaitingDecision
        || gate.revision != form.revision
        || target.as_deref() != Some(form.candidate.as_str())
        || !state.gate_continuations.available(&run_id, &session)
    {
        return command_error(
            graft,
            PatchStatus::Conflict,
            "That gate page is stale. Reload it.",
        );
    }
    let Ok(diff) = crate::workflows::artefacts::CandidateDiff::load(
        &run,
        &gate.diff_base,
        &gate.candidate,
        &state.workflow_artefacts,
    ) else {
        return command_error(
            graft,
            PatchStatus::Conflict,
            "The immutable candidate diff is unavailable.",
        );
    };

    let leases = if matches!(action, DecisionAction::Approve) {
        let Ok(execution) = state.workflow_execution.acquire() else {
            return command_error(
                graft,
                PatchStatus::Conflict,
                "Another workflow is active. Try again.",
            );
        };
        let Ok(agent) = state.agent_leases.acquire(run.agent_id) else {
            return command_error(
                graft,
                PatchStatus::Conflict,
                "That agent is active. Try again.",
            );
        };
        Some((agent, execution))
    } else {
        None
    };
    let Some(continuation) = state.gate_continuations.take(&run_id) else {
        return command_error(graft, PatchStatus::Conflict, "That gate is unavailable.");
    };
    if continuation.session_id != session {
        state.gate_continuations.put_back(continuation);
        return command_error(graft, PatchStatus::Conflict, "That gate is unavailable.");
    }

    if matches!(action, DecisionAction::Cancel) {
        let result = state.workflow_runs.mutate(&run_id, |run| {
            run.cancel_gate(gate_id, form.revision, crate::workflows::now_ms())
        });
        if result.is_err() {
            state.gate_continuations.put_back(continuation);
            return command_error(
                graft,
                PatchStatus::Conflict,
                "That gate page is stale. Reload it.",
            );
        }
        let _ = state.sessions.fail_turn(
            &session,
            &continuation.conversation_key(),
            &continuation.job.id(),
            String::new(),
        );
        continuation.job.finish(JobStatus::Cancelled, None);
        return Ok(hypergraft::outcome::redirect(
            graft,
            format!("/runs/{}", run_id.as_hex()),
        )?);
    }

    let kind = if matches!(action, DecisionAction::Approve) {
        crate::workflows::gates::HumanDecisionKind::Approved
    } else {
        crate::workflows::gates::HumanDecisionKind::RevisionRequested
    };
    let secret = match continuation.connection.auth {
        crate::providers::AuthMethod::ApiKey => Some(continuation.connection.api_key.expose()),
        crate::providers::AuthMethod::Plan => None,
    };
    let decided_at = crate::workflows::now_ms();
    let encoded = crate::workflows::artefacts::encode_human_decision(
        diff.target,
        diff.base,
        kind,
        form.note.as_deref(),
        decided_at,
        secret,
    );
    let Ok((bytes, object_hash, artefact_hash)) = encoded else {
        state.gate_continuations.put_back(continuation);
        return command_error(
            graft,
            PatchStatus::UnprocessableEntity,
            "That revision note is not valid.",
        );
    };
    if state.workflow_artefacts.publish(&bytes) != Ok(object_hash) {
        state.gate_continuations.put_back(continuation);
        return command_error(
            graft,
            PatchStatus::Conflict,
            "Power Plant could not store the decision. Try again.",
        );
    }
    let Some(record) = decision_record(
        &run,
        gate,
        kind,
        decided_at,
        object_hash,
        artefact_hash,
        bytes.len() as u64,
    ) else {
        state.gate_continuations.put_back(continuation);
        return command_error(graft, PatchStatus::Conflict, "That gate is unavailable.");
    };
    let changed = state.workflow_runs.mutate(&run_id, |run| {
        run.decide_gate(gate_id, form.revision, record, kind, decided_at)
    });
    let Ok(changed) = changed else {
        state.gate_continuations.put_back(continuation);
        return command_error(
            graft,
            PatchStatus::Conflict,
            "That gate page is stale. Reload it.",
        );
    };

    if let Some((agent, execution)) = leases {
        if changed.is_terminal() {
            crate::workflows::settle_completed_job(&state, &continuation);
        } else {
            continuation.job.resume();
            tokio::spawn(crate::workflows::execute_run(
                state.clone(),
                continuation,
                agent,
                execution,
            ));
        }
    } else {
        let note = form.note.unwrap_or_default();
        let _ = state.sessions.fail_turn(
            &session,
            &continuation.conversation_key(),
            &continuation.job.id(),
            note,
        );
        continuation
            .job
            .finish(JobStatus::Failed, Some("Revision requested"));
    }
    Ok(hypergraft::outcome::redirect(
        graft,
        format!("/runs/{}", run_id.as_hex()),
    )?)
}

fn decision_record(
    run: &crate::workflows::WorkflowRun,
    gate: &crate::workflows::gates::HumanGateRecord,
    decision: crate::workflows::gates::HumanDecisionKind,
    at: u64,
    object_hash: crate::workflows::artefacts::ObjectHash,
    artefact_hash: crate::workflows::artefacts::ArtefactHash,
    bytes: u64,
) -> Option<crate::workflows::artefacts::ArtefactRecord> {
    let step = run.pinned.definition.step(&gate.step)?;
    let mut inputs = Vec::new();
    for input in &step.inputs {
        let reference = match &input.source {
            crate::workflows::definition::ArtefactSource::RunInitialCandidate => {
                match &run.source {
                    crate::workflows::RunSource::Captured { source } => source.initial.clone(),
                    _ => return None,
                }
            }
            crate::workflows::definition::ArtefactSource::RunCurrentCandidate => {
                match &run.source {
                    crate::workflows::RunSource::Captured { source } => source.accepted.clone(),
                    _ => return None,
                }
            }
            crate::workflows::definition::ArtefactSource::StepOutput { step, output } => run
                .attempts
                .iter()
                .rev()
                .find(|attempt| attempt.step == *step)
                .and_then(|attempt| attempt.outputs.iter().find(|item| item.key == *output))
                .map(|item| item.artefact.clone())
                .or_else(|| {
                    run.gates
                        .iter()
                        .rev()
                        .find(|item| item.step == *step && item.output == *output)
                        .and_then(|item| item.decision.clone())
                })?,
        };
        if !inputs.contains(&reference) {
            inputs.push(reference);
        }
    }
    if !inputs.contains(&gate.diff_base) {
        inputs.push(gate.diff_base.clone());
    }
    Some(crate::workflows::artefacts::ArtefactRecord {
        id: crate::workflows::ArtefactId::generate().ok()?,
        kind: crate::workflows::definition::ArtefactKind::HumanDecision,
        artefact_hash,
        object_hash,
        payload_bytes: bytes,
        created_at_ms: at,
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id: run.id,
            producer: crate::workflows::artefacts::ArtefactProducer::HumanGate {
                gate_id: gate.id,
                step: gate.step.clone(),
                output: gate.output.clone(),
            },
            inputs,
        },
        summary: crate::workflows::artefacts::ArtefactSummary::HumanDecision {
            candidate: run.artefact(&gate.candidate.id)?.candidate_hash()?,
            diff_base: run.artefact(&gate.diff_base.id)?.candidate_hash()?,
            decision,
        },
    })
}

fn command_error(
    graft: CommandGraft,
    status: PatchStatus,
    message: &'static str,
) -> AppResult<Response> {
    #[derive(askama::Template)]
    #[template(
        source = "<div role=\"alert\" class=\"alert alert-error\"><span>{{ message }}</span></div>",
        ext = "html"
    )]
    struct ErrorView {
        message: &'static str,
    }
    let view = ErrorView { message };
    match graft {
        CommandGraft::Document => Ok((status.status_code(), message).into_response()),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "gate-detail",
            &view,
        )?),
    }
}

async fn object(
    State(state): State<AppState>,
    _session: RequiredSession,
    Path((run_raw, gate_raw, side, change)): Path<(String, String, String, String)>,
    headers: HeaderMap,
) -> Response {
    if headers.contains_key(header::RANGE)
        || headers.contains_key(header::IF_MATCH)
        || headers.contains_key(header::IF_NONE_MATCH)
        || headers.contains_key(header::IF_MODIFIED_SINCE)
        || headers.contains_key(header::IF_UNMODIFIED_SINCE)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some((run_id, gate_id)) = ids(&run_raw, &gate_raw) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(index) = change
        .parse::<usize>()
        .ok()
        .filter(|_| !change.starts_with('+') && !change.starts_with('-'))
    else {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    };
    let Some(run) = state.workflow_runs.get(&run_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(gate) = run.gates.iter().find(|item| item.id == gate_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(diff) = crate::workflows::artefacts::CandidateDiff::load(
        &run,
        &gate.diff_base,
        &gate.candidate,
        &state.workflow_artefacts,
    ) else {
        return StatusCode::CONFLICT.into_response();
    };
    let Ok((filename, bytes)) = diff.object(index, &side, &state.workflow_artefacts) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let filename: String = filename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

use axum::response::IntoResponse;
