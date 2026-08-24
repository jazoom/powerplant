use super::{ProviderError, ProviderKind, SecretString, rig::classify_verify_status};

#[test]
fn parses_known_providers() {
    assert_eq!(ProviderKind::parse("xai"), Some(ProviderKind::Xai));
    assert_eq!(
        ProviderKind::parse("openai-codex"),
        Some(ProviderKind::OpenaiCodex)
    );
    assert_eq!(
        ProviderKind::parse("synthetic"),
        Some(ProviderKind::Synthetic)
    );
    assert_eq!(ProviderKind::parse("openai"), None);
}

#[test]
fn secret_debug_is_redacted() {
    let secret = SecretString::new("sk-secret-value".to_owned());
    let debug = format!("{secret:?}");
    assert_eq!(debug, "SecretString(<redacted>)");
    assert!(!debug.contains("sk-secret"));
}

#[test]
fn verify_status_treats_auth_failures_as_rejection() {
    assert_eq!(classify_verify_status(401), Err(ProviderError::Rejected));
    assert_eq!(classify_verify_status(403), Err(ProviderError::Rejected));
    assert_eq!(classify_verify_status(500), Err(ProviderError::Rejected));
}

#[test]
fn verify_status_accepts_success_and_authenticated_request_errors() {
    assert_eq!(classify_verify_status(200), Ok(()));
    assert_eq!(classify_verify_status(400), Ok(()));
    assert_eq!(classify_verify_status(422), Ok(()));
}
