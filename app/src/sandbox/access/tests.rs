use super::network_policy;
use crate::agents::NetworkAccess;

#[test]
fn no_network_policy_denies_all_traffic() {
    let policy = network_policy(&NetworkAccess::None);
    assert!(policy.rules.is_empty());
    assert_eq!(policy.default_egress, microsandbox::NetworkAction::Deny);
    assert_eq!(policy.default_ingress, microsandbox::NetworkAction::Deny);
}

#[test]
fn restricted_policy_allows_only_domain_suffixes_and_dns() {
    let policy = network_policy(&NetworkAccess::Restricted(vec![
        "npmjs.org".to_owned(),
        "github.com".to_owned(),
    ]));
    assert_eq!(policy.default_egress, microsandbox::NetworkAction::Deny);
    assert_eq!(policy.default_ingress, microsandbox::NetworkAction::Deny);
    let rules = format!("{:?}", policy.rules);
    assert!(rules.contains("npmjs.org"));
    assert!(rules.contains("github.com"));
    assert!(rules.contains("DomainSuffix"));
    assert_eq!(policy.rules.len(), 3);
}

#[test]
fn public_policy_allows_public_destinations_but_not_host_or_private_groups() {
    let policy = network_policy(&NetworkAccess::Public);
    assert_eq!(policy.default_egress, microsandbox::NetworkAction::Deny);
    assert_eq!(policy.default_ingress, microsandbox::NetworkAction::Deny);
    let rules = format!("{:?}", policy.rules);
    assert!(rules.contains("Public"));
    assert!(!rules.contains("Group(Private)"));
    assert_eq!(policy.rules.len(), 2);
}
