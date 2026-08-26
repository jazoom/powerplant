use super::{GuestAccess, provider_policy};
use crate::providers::{AuthMethod, ProviderConnection, ProviderKind};

#[test]
fn guest_hosts_are_the_selected_provider() {
    assert_eq!(ProviderKind::Xai.guest_host(AuthMethod::ApiKey), "api.x.ai");
    assert_eq!(
        ProviderKind::Xai.guest_host(AuthMethod::Plan),
        "cli-chat-proxy.grok.com"
    );
    assert_eq!(
        ProviderKind::OpenaiCodex.guest_host(AuthMethod::ApiKey),
        "api.openai.com"
    );
    assert_eq!(
        ProviderKind::Synthetic.guest_host(AuthMethod::ApiKey),
        "api.synthetic.new"
    );
    assert_eq!(
        ProviderKind::Openrouter.guest_host(AuthMethod::ApiKey),
        "openrouter.ai"
    );
    assert_eq!(
        ProviderKind::Deepseek.guest_host(AuthMethod::ApiKey),
        "api.deepseek.com"
    );
}

#[test]
fn api_key_access_keeps_the_secret_for_placeholder_injection() {
    let access = GuestAccess::from_connection(&ProviderConnection::with_key(
        ProviderKind::Xai,
        "sk-test",
        "grok-4.6",
    ));
    assert_eq!(access.host, "api.x.ai");
    assert_eq!(
        access.secret.as_ref().map(|secret| secret.expose()),
        Some("sk-test")
    );
}

#[test]
fn plan_access_does_not_export_a_guest_secret() {
    let access = GuestAccess::from_connection(&ProviderConnection::with_plan(
        ProviderKind::Xai,
        "grok-4.6",
        None,
    ));
    assert_eq!(access.host, "cli-chat-proxy.grok.com");
    assert!(access.secret.is_none());
}

#[test]
fn provider_policy_denies_other_hosts() {
    let policy = provider_policy("api.x.ai");
    assert_eq!(policy.default_egress, microsandbox::NetworkAction::Deny);
    assert_eq!(policy.default_ingress, microsandbox::NetworkAction::Deny);
    let rules = format!("{:?}", policy.rules);
    assert!(rules.contains("api.x.ai"));
    assert!(policy.rules.len() >= 2);
}

#[test]
fn empty_host_denies_all_traffic() {
    let policy = provider_policy("");
    assert!(policy.rules.is_empty());
    assert_eq!(policy.default_egress, microsandbox::NetworkAction::Deny);
}
