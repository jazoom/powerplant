use askama::Template;

use crate::providers::{ProviderError, ProviderKind};

use super::forms::{ConnectField, ConnectForm, FieldError};

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
    pub(super) error: Option<FieldError>,
    pub(super) lifetime_hours: u64,
}

impl ConnectViewModel {
    pub(super) fn initial() -> Self {
        Self::new(options(None), String::new(), None)
    }

    pub(super) fn invalid(form: ConnectForm, error: FieldError) -> Self {
        let selected = form.provider_kind();
        let model = if form.model_is_bounded() {
            form.model
        } else {
            String::new()
        };
        Self::new(options(selected), model, Some(error))
    }

    pub(super) fn failed(kind: ProviderKind, model: String, error: ProviderError) -> Self {
        let field = match error {
            ProviderError::Rejected => ConnectField::ApiKey,
            _ => ConnectField::Provider,
        };
        Self::new(
            options(Some(kind)),
            model,
            Some(FieldError {
                field,
                message: error.message(),
            }),
        )
    }

    pub(super) fn card_contents(&self) -> ConnectCardContents<'_> {
        ConnectCardContents {
            providers: &self.providers,
            model: &self.model,
            error: self.error,
            lifetime_hours: self.lifetime_hours,
        }
    }

    fn new(providers: Vec<ProviderOption>, model: String, error: Option<FieldError>) -> Self {
        Self {
            providers,
            model,
            error,
            lifetime_hours: crate::sessions::SESSION_LIFETIME_HOURS,
        }
    }
}

#[derive(Template)]
#[template(path = "connect/templates/connect.html", block = "card_contents")]
pub(super) struct ConnectCardContents<'a> {
    providers: &'a [ProviderOption],
    model: &'a str,
    error: Option<FieldError>,
    lifetime_hours: u64,
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
