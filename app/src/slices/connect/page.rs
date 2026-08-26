use askama::Template;

use crate::{
    providers::{ProviderError, ProviderKind},
    vault::ProviderVault,
};

use super::forms::{ConnectField, ConnectForm, FieldError};

pub(super) const DOCUMENT_TITLE: &str = "Connect | Power Plant";

pub(super) struct ProviderOption {
    pub(super) value: &'static str,
    pub(super) label: &'static str,
    pub(super) selected: bool,
}

pub(super) struct StoredProviderView {
    pub(super) value: &'static str,
    pub(super) label: &'static str,
}

#[derive(Template)]
#[template(path = "connect/templates/connect.html")]
pub(super) struct ConnectViewModel {
    pub(super) providers: Vec<ProviderOption>,
    pub(super) stored: Vec<StoredProviderView>,
    pub(super) open_chat: bool,
    pub(super) error: Option<FieldError>,
}

impl ConnectViewModel {
    pub(super) fn initial(vault: &ProviderVault) -> Self {
        Self::new(vault, options(None), None)
    }

    pub(super) fn invalid(vault: &ProviderVault, form: ConnectForm, error: FieldError) -> Self {
        Self::new(vault, options(form.provider_kind()), Some(error))
    }

    pub(super) fn failed(vault: &ProviderVault, kind: ProviderKind, error: ProviderError) -> Self {
        let field = match &error {
            ProviderError::Rejected => ConnectField::ApiKey,
            _ => ConnectField::Provider,
        };
        Self::new(
            vault,
            options(Some(kind)),
            Some(FieldError {
                field,
                message: error.message().to_owned(),
            }),
        )
    }

    pub(super) fn card_contents(&self) -> ConnectCardContents<'_> {
        ConnectCardContents {
            providers: &self.providers,
            stored: &self.stored,
            open_chat: self.open_chat,
            error: self.error.as_ref(),
        }
    }

    fn new(
        vault: &ProviderVault,
        providers: Vec<ProviderOption>,
        error: Option<FieldError>,
    ) -> Self {
        let stored = stored_providers(vault);
        let open_chat = !stored.is_empty();
        Self {
            providers,
            stored,
            open_chat,
            error,
        }
    }
}

#[derive(Template)]
#[template(path = "connect/templates/connect.html", block = "card_contents")]
pub(super) struct ConnectCardContents<'a> {
    providers: &'a [ProviderOption],
    stored: &'a [StoredProviderView],
    open_chat: bool,
    error: Option<&'a FieldError>,
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

fn stored_providers(vault: &ProviderVault) -> Vec<StoredProviderView> {
    vault
        .desk_providers()
        .into_iter()
        .map(|provider| StoredProviderView {
            value: provider.kind.as_str(),
            label: provider.kind.label(),
        })
        .collect()
}
