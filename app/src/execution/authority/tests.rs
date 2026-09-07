use crate::{
    agents::{NetworkAccess, ToolId},
    providers::{ModelSelection, ProviderKind},
};

use super::ProjectFreeAuthority;

#[test]
fn empty_host_grants_resolve_only_the_private_workspace() {
    let settings = crate::execution::ExecutionSettings::new(
        ModelSelection::new(ProviderKind::Xai, "test-model".to_owned(), None).unwrap(),
        String::new(),
        vec![ToolId::List, ToolId::Write],
    )
    .and_then(|settings| settings.with_network(NetworkAccess::Public))
    .unwrap();
    let authority = ProjectFreeAuthority::from_settings(7, &settings);

    assert!(authority.policy.grants().is_empty());
    assert_eq!(authority.policy.primary_guest(), "/workspace");
    assert!(authority.policy.resolve("notes.txt").is_ok());
    assert!(authority.policy.resolve("/project/secret").is_err());
    assert_eq!(authority.network, NetworkAccess::Public);
}
