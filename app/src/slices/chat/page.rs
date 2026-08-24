use askama::Template;

use crate::{
    markdown,
    providers::{ChatTurn, ProviderConnection, Role},
    sessions::SessionSnapshot,
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
}

impl ChatViewModel {
    pub(super) fn from_session(session: &SessionSnapshot, error: &'static str) -> Self {
        Self::from_parts(&session.connection, &session.turns, error)
    }

    pub(super) fn from_parts(
        connection: &ProviderConnection,
        turns: &[ChatTurn],
        error: &'static str,
    ) -> Self {
        Self {
            provider_label: connection.kind.label(),
            model: connection.model.clone(),
            turns: turns
                .iter()
                .enumerate()
                .map(|(index, turn)| turn_view(index, turn))
                .collect(),
            error,
        }
    }

    pub(super) fn transcript(&self) -> TranscriptContents<'_> {
        TranscriptContents { turns: &self.turns }
    }

    pub(super) fn composer(&self) -> ComposerContents<'_> {
        ComposerContents { error: self.error }
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
