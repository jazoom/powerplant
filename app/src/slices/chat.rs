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
    error::AppResult,
    responses,
    sessions::{self, BeginTurnError, JobIdError, OptionalSession, SessionSnapshot},
    state::AppState,
};

use self::{
    forms::{ChatForm, CursorError, ObserveQuery},
    job::{observe_response, run_job, user_transcript_patch},
    page::{ChatViewModel, ComposerContents, TranscriptContents},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(show).post(send))
        .route("/jobs/{job_id}/cancel", post(cancel))
}

async fn show(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: GraftRequest,
    Query(query): Query<ObserveQuery>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };

    match graft {
        GraftRequest::Document => render_document(&state, PatchStatus::Ok, view(&session, "")),
        GraftRequest::Navigation => navigate_page(&state, &view(&session, "")),
        GraftRequest::Patch => observe(&state, &session, query),
    }
}

async fn send(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Form(form): Form<ChatForm>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };

    if !form.is_bounded() {
        return reject_chat_input(&state, graft, &session, "Enter a message.");
    }

    let started = match state
        .sessions
        .begin_turn(&session.id, form.message.trim().to_owned())
    {
        Ok(started) => started,
        Err(BeginTurnError::MissingSession) => {
            return Ok(responses::graft_redirect(graft, "/connect"));
        }
        Err(BeginTurnError::Conflict) => {
            let Some(latest) = state.sessions.snapshot(&session.id) else {
                return Ok(responses::graft_redirect(graft, "/connect"));
            };
            return reject_parallel_command(&state, graft, &latest);
        }
        Err(BeginTurnError::JobId) => {
            return Err(crate::error::AppError::new(
                "create job identifier",
                JobIdError::RandomUnavailable,
            ));
        }
    };

    let job = started.job.clone();
    tokio::spawn(run_job(
        state.clone(),
        session.id,
        started.connection,
        started.turns.clone(),
        job,
    ));

    match graft {
        CommandGraft::Document => {
            let Some(latest) = state.sessions.snapshot(&session.id) else {
                return Ok(responses::graft_redirect(graft, "/connect"));
            };
            render_document(&state, PatchStatus::Ok, view(&latest, ""))
        }
        CommandGraft::Patch => accept_job_patch(&started.turns, &started.job.id().as_hex()),
    }
}

async fn cancel(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: CommandGraft,
    Path(job_id): Path<String>,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    if let Some(id) = sessions::JobId::parse(&job_id)
        && let Some(job) = state.sessions.job(&session.id, &id)
    {
        job.request_cancel();
    }
    let Some(latest) = state.sessions.snapshot(&session.id) else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    match graft {
        CommandGraft::Document => render_document(&state, PatchStatus::Ok, view(&latest, "")),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "composer",
            &view(&latest, "").composer(),
        )?),
    }
}

fn observe(
    state: &AppState,
    session: &SessionSnapshot,
    query: ObserveQuery,
) -> AppResult<Response> {
    let cursor = match query.cursor() {
        Ok(cursor) => cursor,
        Err(CursorError::Malformed | CursorError::Excessive) => {
            return Ok(hypergraft::outcome::children_patch(
                PatchStatus::UnprocessableEntity,
                "composer",
                &view(session, "That cursor is not valid.").composer(),
            )?);
        }
    };
    let Some(job_id) = query.job_id() else {
        return refresh_composer(session);
    };
    let Some(job) = state.sessions.job(&session.id, &job_id) else {
        return refresh_composer(session);
    };
    Ok(observe_response(job, cursor))
}

fn refresh_composer(session: &SessionSnapshot) -> AppResult<Response> {
    Ok(hypergraft::outcome::children_patch(
        PatchStatus::Ok,
        "composer",
        &view(session, "").composer(),
    )?)
}

fn accept_job_patch(turns: &[crate::providers::ChatTurn], job_id: &str) -> AppResult<Response> {
    let mut patches = user_transcript_patch(turns)?;
    patches.children(
        "composer",
        &ComposerContents::observing(job_id, 0, "Writing", ""),
    )?;
    Ok(patches.respond(PatchStatus::Ok)?)
}

fn reject_parallel_command(
    state: &AppState,
    graft: CommandGraft,
    session: &SessionSnapshot,
) -> AppResult<Response> {
    const MESSAGE: &str = "Wait until this reply finishes.";
    match graft {
        CommandGraft::Document => {
            render_document(state, PatchStatus::Conflict, view(session, MESSAGE))
        }
        CommandGraft::Patch => {
            let view = view(session, MESSAGE);
            let mut patches = PatchSet::new();
            patches.children("transcript", &TranscriptContents { turns: &view.turns })?;
            patches.children("composer", &view.composer())?;
            Ok(patches.respond(PatchStatus::Conflict)?)
        }
    }
}

fn reject_chat_input(
    state: &AppState,
    graft: CommandGraft,
    session: &SessionSnapshot,
    message: &'static str,
) -> AppResult<Response> {
    match graft {
        CommandGraft::Document => render_document(
            state,
            PatchStatus::UnprocessableEntity,
            view(session, message),
        ),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::UnprocessableEntity,
            "composer",
            &view(session, message).composer(),
        )?),
    }
}

fn view(session: &SessionSnapshot, error: &'static str) -> ChatViewModel {
    ChatViewModel::from_session(session, error)
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
