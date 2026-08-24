use askama::Template;

use crate::{
    markdown,
    providers::{ChatTurn, ProviderConnection, Role},
    sessions::{JobSnapshot, JobStatus, SessionSnapshot},
};

pub(super) const DOCUMENT_TITLE: &str = "Chat | Circus";

pub(super) struct TurnView {
    pub(super) id: String,
    pub(super) is_user: bool,
    pub(super) html: String,
}

#[derive(Template)]
#[template(path = "chat/templates/chat.html")]
pub(super) struct ChatViewModel {
    pub(super) provider_label: &'static str,
    pub(super) model: String,
    pub(super) turns: Vec<TurnView>,
    pub(super) error: &'static str,
    pub(super) job_id: String,
    pub(super) cursor: u64,
    pub(super) job_active: bool,
    pub(super) job_status: &'static str,
}

impl ChatViewModel {
    pub(super) fn from_session(session: &SessionSnapshot, error: &'static str) -> Self {
        Self::from_parts(
            &session.connection,
            &session.turns,
            session.job.as_ref(),
            error,
        )
    }

    pub(super) fn from_parts(
        connection: &ProviderConnection,
        turns: &[ChatTurn],
        job: Option<&JobSnapshot>,
        error: &'static str,
    ) -> Self {
        let mut views: Vec<TurnView> = turns
            .iter()
            .enumerate()
            .map(|(index, turn)| turn_view(index, turn))
            .collect();
        let mut job_id = String::new();
        let mut cursor = 0;
        let mut job_active = false;
        let mut job_status = "";
        let mut error = error;
        if let Some(job) = job {
            if !job.output.is_empty() && views.len() == job.assistant_index {
                views.push(assistant_turn(job.assistant_index, &job.output));
            }
            if job.status == JobStatus::Running {
                job_id = job.id.as_hex();
                cursor = job.latest_seq;
                job_active = true;
                job_status = if job.cancel_requested {
                    "Stopping"
                } else {
                    "Writing"
                };
            } else if error.is_empty()
                && let Some(message) = job.error
            {
                error = message;
            }
        }
        Self {
            provider_label: connection.kind.label(),
            model: connection.model.clone(),
            turns: views,
            error,
            job_id,
            cursor,
            job_active,
            job_status,
        }
    }

    pub(super) fn composer(&self) -> ComposerContents<'_> {
        ComposerContents {
            error: self.error,
            job_id: &self.job_id,
            cursor: self.cursor,
            job_active: self.job_active,
            job_status: self.job_status,
        }
    }
}

pub(super) fn user_turn(index: usize, text: &str) -> TurnView {
    TurnView {
        id: turn_id(index),
        is_user: true,
        html: format!("<p>{}</p>", markdown::escape_plain(text)),
    }
}

pub(super) fn assistant_turn(index: usize, text: &str) -> TurnView {
    TurnView {
        id: turn_id(index),
        is_user: false,
        html: markdown::render(text),
    }
}

pub(super) fn turn_id(index: usize) -> String {
    format!("turn-{}", index + 1)
}

#[derive(Template)]
#[template(path = "chat/templates/chat.html", block = "transcript")]
pub(super) struct TranscriptContents<'a> {
    pub(super) turns: &'a [TurnView],
}

#[derive(Template)]
#[template(path = "chat/templates/chat.html", block = "composer")]
pub(super) struct ComposerContents<'a> {
    pub(super) error: &'a str,
    pub(super) job_id: &'a str,
    pub(super) cursor: u64,
    pub(super) job_active: bool,
    pub(super) job_status: &'static str,
}

impl ComposerContents<'static> {
    pub(super) fn idle(error: &'static str) -> Self {
        Self {
            error,
            job_id: "",
            cursor: 0,
            job_active: false,
            job_status: "",
        }
    }
}

impl<'a> ComposerContents<'a> {
    pub(super) fn observing(
        job_id: &'a str,
        cursor: u64,
        status: &'static str,
        error: &'a str,
    ) -> Self {
        Self {
            error,
            job_id,
            cursor,
            job_active: true,
            job_status: status,
        }
    }
}

#[derive(Template)]
#[template(path = "chat/templates/job_cursor.html")]
pub(super) struct JobCursorContents<'a> {
    pub(super) job_id: &'a str,
    pub(super) cursor: u64,
}

#[derive(Template)]
#[template(path = "chat/templates/turn_article.html")]
pub(super) struct TurnArticle<'a> {
    pub(super) turn: &'a TurnView,
}

#[derive(Template)]
#[template(path = "chat/templates/turn_body.html")]
pub(super) struct TurnBody<'a> {
    pub(super) turn: &'a TurnView,
}

fn turn_view(index: usize, turn: &ChatTurn) -> TurnView {
    match turn.role {
        Role::User => user_turn(index, &turn.text),
        Role::Assistant => assistant_turn(index, &turn.text),
    }
}
