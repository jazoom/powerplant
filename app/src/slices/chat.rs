mod forms;
mod page;

#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use axum::{Form, Router, extract::State, response::Response, routing::get};
use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::{
    error::AppResult,
    providers::{ChatTurn, ProviderError, Role},
    responses::{self, CommandGraft, PageGraft, PatchSet, PatchStatus},
    sessions::{OptionalSession, SessionId, SessionSnapshot},
    state::AppState,
};

use self::{
    forms::ChatForm,
    page::{
        ChatViewModel, ComposerContents, TranscriptContents, TurnArticle, TurnBody, TurnView,
        assistant_turn, user_turn,
    },
};

const MIN_PROGRESS_INTERVAL: Duration = Duration::from_millis(48);
const MIN_PROGRESS_CHARS: usize = 32;

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/", get(show).post(send))
}

async fn show(
    State(state): State<AppState>,
    OptionalSession(session): OptionalSession,
    graft: PageGraft,
) -> AppResult<Response> {
    let Some(session) = session else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    let view = ChatViewModel::from_session(&session, "");
    responses::chat_graft_page(graft, page::DOCUMENT_TITLE, &state.assets, &view)
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
        return render_chat(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            ChatViewModel::from_session(&session, "Enter a message."),
        );
    }

    let session_for_stream = session.clone();
    let mut turns = session.turns;
    turns.push(ChatTurn {
        role: Role::User,
        text: form.message.trim().to_owned(),
    });
    if !state.sessions.replace_turns(&session.id, turns.clone()) {
        return Ok(responses::graft_redirect(graft, "/connect"));
    }

    match graft {
        CommandGraft::Document => match state.chat.complete(&session.connection, &turns).await {
            Ok(reply) => {
                turns.push(ChatTurn {
                    role: Role::Assistant,
                    text: reply,
                });
                if !state.sessions.replace_turns(&session.id, turns.clone()) {
                    return Ok(responses::graft_redirect(graft, "/connect"));
                }
                render_chat(
                    &state,
                    graft,
                    PatchStatus::Ok,
                    ChatViewModel::from_parts(&session.connection, &turns, ""),
                )
            }
            Err(error) => render_chat(
                &state,
                graft,
                provider_status(error),
                ChatViewModel::from_parts(&session.connection, &turns, error.message()),
            ),
        },
        CommandGraft::Patch => Ok(stream_chat(state, session_for_stream, turns)),
    }
}

fn stream_chat(state: AppState, session: SessionSnapshot, turns: Vec<ChatTurn>) -> Response {
    let (tx, rx) = mpsc::channel::<hypergraft::StreamFrame>(4);
    tokio::spawn(run_stream(tx, state, session, turns));
    let frames = futures_util::stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|item| (item, rx))
    });
    hypergraft::outcome::stream_response(frames)
}

async fn run_stream(
    tx: mpsc::Sender<hypergraft::StreamFrame>,
    state: AppState,
    session: SessionSnapshot,
    turns: Vec<ChatTurn>,
) {
    let user_index = turns.len() - 1;
    let assistant_index = turns.len();
    if !send_user_progress(&tx, &turns, user_index).await {
        return;
    }

    let mut tokens = match state.chat.stream(&session.connection, &turns).await {
        Ok(stream) => stream,
        Err(error) => {
            let _ = send_final(&tx, None, false, provider_status(error), error.message()).await;
            return;
        }
    };

    let mut reply = String::new();
    let mut last_emit = Instant::now();
    let mut last_len = 0;
    let mut assistant_visible = false;

    while let Some(chunk) = tokens.next().await {
        let piece = match chunk {
            Ok(text) => text,
            Err(error) => {
                persist_partial(&state, &session.id, &turns, &reply);
                let assistant = assistant_visible.then(|| assistant_turn(assistant_index, &reply));
                let _ = send_final(
                    &tx,
                    assistant,
                    assistant_visible,
                    provider_status(error),
                    error.message(),
                )
                .await;
                return;
            }
        };
        reply.push_str(&piece);
        let due = last_emit.elapsed() >= MIN_PROGRESS_INTERVAL
            || reply.len().saturating_sub(last_len) >= MIN_PROGRESS_CHARS;
        if !due {
            continue;
        }
        if !send_assistant_progress(&tx, assistant_index, &reply, assistant_visible).await {
            return;
        }
        assistant_visible = true;
        last_emit = Instant::now();
        last_len = reply.len();
    }

    if reply.trim().is_empty() {
        let _ = send_final(
            &tx,
            None,
            false,
            provider_status(ProviderError::EmptyReply),
            ProviderError::EmptyReply.message(),
        )
        .await;
        return;
    }

    let mut stored = turns;
    stored.push(ChatTurn {
        role: Role::Assistant,
        text: reply.clone(),
    });
    if !state.sessions.replace_turns(&session.id, stored) {
        return;
    }

    let _ = send_final(
        &tx,
        Some(assistant_turn(assistant_index, &reply)),
        assistant_visible,
        PatchStatus::Ok,
        "",
    )
    .await;
}

fn persist_partial(state: &AppState, id: &SessionId, turns: &[ChatTurn], reply: &str) {
    if reply.trim().is_empty() {
        return;
    }
    let mut stored = turns.to_vec();
    stored.push(ChatTurn {
        role: Role::Assistant,
        text: reply.to_owned(),
    });
    let _ = state.sessions.replace_turns(id, stored);
}

async fn send_user_progress(
    tx: &mpsc::Sender<hypergraft::StreamFrame>,
    turns: &[ChatTurn],
    user_index: usize,
) -> bool {
    let Some(user) = turns.get(user_index) else {
        return false;
    };
    let turn = user_turn(user_index, &user.text);
    let frame = if user_index == 0 {
        let view = [turn];
        PatchSet::new()
            .with_children("transcript", &TranscriptContents { turns: &view })
            .and_then(PatchSet::encode_progress)
    } else {
        PatchSet::new()
            .with_append("transcript", &TurnArticle { turn: &turn })
            .and_then(PatchSet::encode_progress)
    };
    send_frame(tx, frame).await
}

async fn send_assistant_progress(
    tx: &mpsc::Sender<hypergraft::StreamFrame>,
    assistant_index: usize,
    text: &str,
    already_visible: bool,
) -> bool {
    let turn = assistant_turn(assistant_index, text);
    let frame = if already_visible {
        PatchSet::new()
            .with_children(&turn.id, &TurnBody { turn: &turn })
            .and_then(PatchSet::encode_progress)
    } else {
        PatchSet::new()
            .with_append("transcript", &TurnArticle { turn: &turn })
            .and_then(PatchSet::encode_progress)
    };
    send_frame(tx, frame).await
}

async fn send_final(
    tx: &mpsc::Sender<hypergraft::StreamFrame>,
    assistant: Option<TurnView>,
    assistant_visible: bool,
    status: PatchStatus,
    error: &'static str,
) -> bool {
    let composer = ComposerContents { error };
    let mut patches = PatchSet::new();
    if let Some(turn) = assistant.as_ref() {
        let result = if assistant_visible {
            patches.children(&turn.id, &TurnBody { turn })
        } else {
            patches.append("transcript", &TurnArticle { turn })
        };
        if result.is_err() {
            return false;
        }
    }
    if patches.children("composer", &composer).is_err() {
        return false;
    }
    send_frame(tx, patches.encode_final(status)).await
}

async fn send_frame(
    tx: &mpsc::Sender<hypergraft::StreamFrame>,
    frame: Result<hypergraft::StreamFrame, hypergraft::PatchBuildError>,
) -> bool {
    let Ok(frame) = frame else {
        return false;
    };
    tx.send(frame).await.is_ok()
}

fn provider_status(error: ProviderError) -> PatchStatus {
    match error {
        ProviderError::Rejected => PatchStatus::Unauthorized,
        ProviderError::Unreachable | ProviderError::EmptyReply => PatchStatus::UnprocessableEntity,
    }
}

fn render_chat(
    state: &AppState,
    graft: CommandGraft,
    status: PatchStatus,
    view: ChatViewModel,
) -> AppResult<Response> {
    match graft {
        CommandGraft::Document => {
            let mut response =
                responses::chat_page_response(page::DOCUMENT_TITLE, &state.assets, &view)?;
            *response.status_mut() = match status {
                PatchStatus::Ok => axum::http::StatusCode::OK,
                PatchStatus::Unauthorized => axum::http::StatusCode::UNAUTHORIZED,
                PatchStatus::Conflict => axum::http::StatusCode::CONFLICT,
                PatchStatus::UnprocessableEntity => axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                PatchStatus::TooManyRequests(_) => axum::http::StatusCode::TOO_MANY_REQUESTS,
            };
            Ok(response)
        }
        CommandGraft::Patch => {
            let mut patches = PatchSet::new();
            patches.children("transcript", &view.transcript())?;
            patches.children("composer", &view.composer())?;
            Ok(patches.respond(status)?)
        }
    }
}
