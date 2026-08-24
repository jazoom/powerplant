use super::{ConnectField, ConnectForm};
use crate::providers::{MAXIMUM_API_KEY_BYTES, ProviderKind};

fn form(provider: &str, api_key: &str) -> ConnectForm {
    ConnectForm {
        provider: provider.to_owned(),
        api_key: api_key.to_owned(),
    }
}

fn failed_field(provider: &str, api_key: &str) -> Option<ConnectField> {
    form(provider, api_key)
        .validate()
        .err()
        .map(|error| error.field)
}

#[test]
fn accepts_a_bounded_key() {
    let form = form("xai", "sk-test-key");
    assert_eq!(form.provider_kind(), Some(ProviderKind::Xai));
    assert_eq!(form.validate(), Ok(ProviderKind::Xai));
}

#[test]
fn rejects_empty_or_oversized_or_control_keys() {
    assert_eq!(failed_field("xai", ""), Some(ConnectField::ApiKey));
    assert_eq!(
        failed_field("xai", &"a".repeat(MAXIMUM_API_KEY_BYTES + 1)),
        Some(ConnectField::ApiKey)
    );
    assert_eq!(
        failed_field("xai", "sk-\u{0000}secret"),
        Some(ConnectField::ApiKey)
    );
}

#[test]
fn rejects_unknown_provider() {
    assert_eq!(form("openai", "sk-test-key").provider_kind(), None);
}

#[test]
fn names_the_failed_field() {
    assert_eq!(
        failed_field("openai", "sk-test-key"),
        Some(ConnectField::Provider)
    );
    assert_eq!(
        failed_field("", "sk-test-key"),
        Some(ConnectField::Provider)
    );
    assert_eq!(failed_field("xai", ""), Some(ConnectField::ApiKey));
    assert_eq!(ConnectField::Provider.id(), "connect-provider");
    assert_eq!(ConnectField::ApiKey.id(), "connect-key");
}
