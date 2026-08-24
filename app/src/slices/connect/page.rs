use askama::Template;

use crate::providers::ProviderKind;

use super::forms::ConnectForm;

pub(super) const DOCUMENT_TITLE: &str = "Connect | Circus";

pub(super) struct ProviderOption {
    pub(super) value: &'static str,
    pub(super) label: &'static str,
    pub(super) selected: bool,
}

#[derive(Template)]
#[template(path = "connect/templates/connect.html")]
pub(super) struct ConnectViewModel {
    pub(super) providers: Vec<ProviderOption>,
    pub(super) model: String,
    pub(super) error: &'static str,
}

impl ConnectViewModel {
    pub(super) fn initial() -> Self {
        Self {
            providers: options(None),
            model: String::new(),
            error: "",
        }
    }

    pub(super) fn invalid(form: ConnectForm, error: &'static str) -> Self {
        Self {
            providers: options(form.provider_kind()),
            model: form.model,
            error,
        }
    }

    pub(super) fn rejected(kind: ProviderKind, model: String) -> Self {
        Self {
            providers: options(Some(kind)),
            model,
            error: "That key was rejected. Check the provider and try again.",
        }
    }

    pub(super) fn card_contents(&self) -> ConnectCardContents<'_> {
        ConnectCardContents {
            providers: &self.providers,
            model: &self.model,
            error: self.error,
        }
    }
}

#[derive(Template)]
#[template(path = "connect/templates/connect.html", block = "card_contents")]
pub(super) struct ConnectCardContents<'a> {
    providers: &'a [ProviderOption],
    model: &'a str,
    error: &'static str,
}

fn options(selected: Option<ProviderKind>) -> Vec<ProviderOption> {
    ProviderKind::ALL
        .into_iter()
        .map(|kind| ProviderOption {
            value: kind.as_str(),
            label: kind.label(),
            selected: selected == Some(kind),
        })
        .collect()
}
