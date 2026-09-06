use askama::Template;

use crate::conversations::ConversationRecord;

pub(super) const CATALOGUE_TITLE: &str = "Conversations | Power Plant";
pub(super) const NEW_TITLE: &str = "New conversation | Power Plant";

pub(super) struct ConversationListItem {
    pub(super) id: String,
    pub(super) title: String,
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "conversation_catalogue"
)]
pub(super) struct CatalogueView {
    pub(super) conversations: Vec<ConversationListItem>,
}

impl CatalogueView {
    pub(super) fn from_records(records: &[ConversationRecord]) -> Self {
        let mut conversations: Vec<_> = records
            .iter()
            .map(|record| ConversationListItem {
                id: record.id.as_hex(),
                title: record.title.clone(),
            })
            .collect();
        conversations.sort_by(|left, right| left.title.cmp(&right.title));
        Self { conversations }
    }
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "conversation_form"
)]
pub(super) struct ConversationFormContents<'a> {
    pub(super) title: &'a str,
    pub(super) error: &'static str,
}

#[derive(Template)]
#[template(path = "conversations/templates/index.html", block = "new_page")]
pub(super) struct ConversationFormView {
    pub(super) title: String,
    pub(super) error: &'static str,
}

impl ConversationFormView {
    pub(super) fn new(title: &str, error: &'static str) -> Self {
        Self {
            title: title.to_owned(),
            error,
        }
    }

    pub(super) fn contents(&self) -> ConversationFormContents<'_> {
        ConversationFormContents {
            title: &self.title,
            error: self.error,
        }
    }
}

#[derive(Template)]
#[template(
    path = "conversations/templates/index.html",
    block = "conversation_detail"
)]
pub(super) struct ConversationDetailContents<'a> {
    pub(super) heading: &'a str,
    pub(super) title: &'a str,
    pub(super) id: &'a str,
    pub(super) revision: &'a str,
    pub(super) error: &'static str,
}

#[derive(Template)]
#[template(path = "conversations/templates/index.html", block = "detail_page")]
pub(super) struct ConversationDetailView {
    pub(super) heading: String,
    pub(super) document_title: String,
    pub(super) title: String,
    pub(super) id: String,
    pub(super) revision: String,
    pub(super) error: &'static str,
}

impl ConversationDetailView {
    pub(super) fn from_record(
        record: &ConversationRecord,
        title: &str,
        error: &'static str,
    ) -> Self {
        Self {
            heading: record.title.clone(),
            document_title: format!("{} | Power Plant", record.title),
            title: title.to_owned(),
            id: record.id.as_hex(),
            revision: record.revision.to_string(),
            error,
        }
    }

    pub(super) fn contents(&self) -> ConversationDetailContents<'_> {
        ConversationDetailContents {
            heading: &self.heading,
            title: &self.title,
            id: &self.id,
            revision: &self.revision,
            error: self.error,
        }
    }
}
