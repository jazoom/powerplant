use askama::Template;

use crate::{
    plan_login::PendingPlan,
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
    pub(super) method: &'static str,
}

pub(super) struct PendingPlanView {
    pub(super) provider_label: &'static str,
    pub(super) verification_uri: String,
    pub(super) user_code: String,
    pub(super) error: Option<String>,
    pub(super) active: bool,
}

#[derive(Template)]
#[template(path = "connect/templates/connect.html")]
pub(super) struct ConnectViewModel {
    pub(super) providers: Vec<ProviderOption>,
    pub(super) stored: Vec<StoredProviderView>,
    pub(super) pending: Option<PendingPlanView>,
    pub(super) has_stored_providers: bool,
    pub(super) clear_api_key: bool,
    pub(super) show_chatgpt: bool,
    pub(super) show_grok: bool,
    pub(super) sandbox_missing: &'static str,
    pub(super) error: Option<FieldError>,
}

impl ConnectViewModel {
    pub(super) fn initial(
        vault: &ProviderVault,
        pending: Option<PendingPlan>,
        sandbox_missing: &'static str,
    ) -> Self {
        Self::new(vault, None, pending, sandbox_missing, None)
    }

    pub(super) fn invalid(
        vault: &ProviderVault,
        pending: Option<PendingPlan>,
        sandbox_missing: &'static str,
        form: ConnectForm,
        error: FieldError,
    ) -> Self {
        Self::new(
            vault,
            form.provider_kind(),
            pending,
            sandbox_missing,
            Some(error),
        )
    }

    pub(super) fn failed(
        vault: &ProviderVault,
        pending: Option<PendingPlan>,
        sandbox_missing: &'static str,
        kind: ProviderKind,
        error: ProviderError,
    ) -> Self {
        let field = match &error {
            ProviderError::Rejected => ConnectField::ApiKey,
            ProviderError::Reauthenticate => ConnectField::Plan,
            _ => ConnectField::Provider,
        };
        Self::new(
            vault,
            Some(kind),
            pending,
            sandbox_missing,
            Some(FieldError {
                field,
                message: error.message().to_owned(),
            }),
        )
    }

    pub(super) fn plan_invalid(
        vault: &ProviderVault,
        pending: Option<PendingPlan>,
        sandbox_missing: &'static str,
        error: FieldError,
    ) -> Self {
        Self::new(vault, None, pending, sandbox_missing, Some(error))
    }

    pub(super) fn clear_api_key(mut self) -> Self {
        self.clear_api_key = true;
        self
    }

    pub(super) fn card_contents(&self) -> ConnectCardContents<'_> {
        ConnectCardContents {
            providers: &self.providers,
            stored: &self.stored,
            pending: self.pending.as_ref(),
            has_stored_providers: self.has_stored_providers,
            clear_api_key: self.clear_api_key,
            show_chatgpt: self.show_chatgpt,
            show_grok: self.show_grok,
            sandbox_missing: self.sandbox_missing,
            error: self.error.as_ref(),
        }
    }

    fn new(
        vault: &ProviderVault,
        selected: Option<ProviderKind>,
        pending: Option<PendingPlan>,
        sandbox_missing: &'static str,
        error: Option<FieldError>,
    ) -> Self {
        let stored = stored_providers(vault);
        let has_stored_providers = !stored.is_empty();
        Self {
            providers: options(vault, selected),
            stored,
            pending: pending.map(pending_view),
            has_stored_providers,
            clear_api_key: false,
            show_chatgpt: !vault.contains(ProviderKind::OpenaiCodex),
            show_grok: !vault.contains(ProviderKind::Xai),
            sandbox_missing,
            error,
        }
    }
}

#[derive(Template)]
#[template(path = "connect/templates/connect.html", block = "card_contents")]
pub(super) struct ConnectCardContents<'a> {
    providers: &'a [ProviderOption],
    stored: &'a [StoredProviderView],
    pending: Option<&'a PendingPlanView>,
    has_stored_providers: bool,
    clear_api_key: bool,
    show_chatgpt: bool,
    show_grok: bool,
    sandbox_missing: &'a str,
    error: Option<&'a FieldError>,
}

fn pending_view(pending: PendingPlan) -> PendingPlanView {
    let active = pending.error.is_none();
    PendingPlanView {
        provider_label: pending.kind.label(),
        verification_uri: pending.verification_uri,
        user_code: pending.user_code,
        active,
        error: pending.error,
    }
}

fn options(vault: &ProviderVault, selected: Option<ProviderKind>) -> Vec<ProviderOption> {
    ProviderKind::ALL
        .into_iter()
        .filter(|kind| !vault.contains(*kind))
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
            method: provider.auth.label(),
        })
        .collect()
}
