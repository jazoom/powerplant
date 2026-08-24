use super::{ConnectField, ConnectForm, MAXIMUM_API_KEY_BYTES, MAXIMUM_MODEL_BYTES};
use crate::providers::ProviderKind;

fn form(provider: &str, api_key: &str, model: &str) -> ConnectForm {
    ConnectForm {
        provider: provider.to_owned(),
        api_key: api_key.to_owned(),
        model: model.to_owned(),
    }
}

fn failed_field(provider: &str, api_key: &str, model: &str) -> Option<ConnectField> {
    form(provider, api_key, model)
        .validate()
        .err()
        .map(|error| error.field)
}

#[test]
fn accepts_a_bounded_key_and_blank_model() {
    let form = form("xai", "sk-test-key", "  ");
    assert_eq!(form.provider_kind(), Some(ProviderKind::Xai));
    assert!(form.api_key_is_bounded());
    assert!(form.model_is_bounded());
    assert_eq!(form.validate(), Ok(ProviderKind::Xai));
    assert_eq!(
        form.resolved_model(ProviderKind::Xai),
        ProviderKind::Xai.default_model()
    );
}

#[test]
fn rejects_empty_or_oversized_or_control_keys() {
    assert!(!form("xai", "", "").api_key_is_bounded());
    assert!(!form("xai", &"a".repeat(MAXIMUM_API_KEY_BYTES + 1), "").api_key_is_bounded());
    assert!(!form("xai", "sk-\u{0000}secret", "").api_key_is_bounded());
}

#[test]
fn rejects_unknown_provider() {
    assert_eq!(form("openai", "sk-test-key", "").provider_kind(), None);
}

#[test]
fn names_the_failed_field() {
    assert_eq!(
        failed_field("openai", "sk-test-key", ""),
        Some(ConnectField::Provider)
    );
    assert_eq!(
        failed_field("", "sk-test-key", ""),
        Some(ConnectField::Provider)
    );
    assert_eq!(
        failed_field("xai", "", "grok-4.6"),
        Some(ConnectField::ApiKey)
    );
    assert_eq!(
        failed_field("xai", "sk-\u{0000}secret", ""),
        Some(ConnectField::ApiKey)
    );
    assert_eq!(
        failed_field("xai", "sk-test-key", &"a".repeat(MAXIMUM_MODEL_BYTES + 1)),
        Some(ConnectField::Model)
    );
    assert_eq!(
        failed_field("xai", "sk-test-key", "grok-\u{0000}"),
        Some(ConnectField::Model)
    );
    assert_eq!(failed_field("xai", "sk-test-key", ""), None);
    assert_eq!(ConnectField::Provider.id(), "connect-provider");
    assert_eq!(ConnectField::ApiKey.id(), "connect-key");
    assert_eq!(ConnectField::Model.id(), "connect-model");
}
