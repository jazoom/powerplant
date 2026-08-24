use std::sync::Arc;

use super::{ModelCatalogue, refresh};
use crate::{
    config::RuntimeConfig,
    providers::{
        ChatBackend, ProviderConnection, ProviderKind, SecretString, scripted::ScriptedBackend,
    },
};

#[test]
fn only_the_latest_refresh_can_replace_a_provider_catalogue() {
    let catalogue = ModelCatalogue::default();
    let first = catalogue.begin_refresh(ProviderKind::Xai);
    catalogue.finish(ProviderKind::Xai, first, Some(vec!["current".to_owned()]));

    let stale = catalogue.begin_refresh(ProviderKind::Xai);
    assert_eq!(catalogue.list(ProviderKind::Xai), ["current"]);
    assert!(catalogue.pending(ProviderKind::Xai));

    let current = catalogue.begin_refresh(ProviderKind::Xai);
    catalogue.finish(ProviderKind::Xai, current, Some(vec!["newer".to_owned()]));
    catalogue.finish(ProviderKind::Xai, stale, Some(vec!["stale".to_owned()]));
    let failed = catalogue.begin_refresh(ProviderKind::Xai);
    catalogue.finish(ProviderKind::Xai, failed, None);

    assert_eq!(catalogue.list(ProviderKind::Xai), ["newer"]);
    assert!(!catalogue.pending(ProviderKind::Xai));
}

#[test]
fn removal_invalidates_an_active_refresh() {
    let catalogue = ModelCatalogue::default();
    let revision = catalogue.begin_refresh(ProviderKind::Xai);

    catalogue.remove(ProviderKind::Xai);
    catalogue.finish(ProviderKind::Xai, revision, Some(vec!["stale".to_owned()]));

    assert!(catalogue.list(ProviderKind::Xai).is_empty());
    assert!(!catalogue.pending(ProviderKind::Xai));
}

#[tokio::test]
async fn a_successful_backend_refresh_stores_the_model_list() {
    let mut state = crate::state::for_test(RuntimeConfig::development_for_test());
    state.chat = Arc::new(ChatBackend::Scripted(
        ScriptedBackend::accept().with_models(vec![
            "hf:moonshotai/Kimi-K3".to_owned(),
            "syn:large:text".to_owned(),
        ]),
    ));
    let connection = ProviderConnection {
        kind: ProviderKind::Synthetic,
        api_key: SecretString::new("test-key".to_owned()),
        model: "hf:moonshotai/Kimi-K3".to_owned(),
    };

    refresh(state.clone(), connection);
    for _ in 0..100 {
        if !state.models.pending(ProviderKind::Synthetic) {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert_eq!(
        state.models.list(ProviderKind::Synthetic),
        ["hf:moonshotai/Kimi-K3", "syn:large:text"]
    );
    assert!(!state.models.pending(ProviderKind::Synthetic));
}
