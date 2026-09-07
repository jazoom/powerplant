use super::*;
use crate::{agents::NetworkAccess, config::RuntimeConfig};
use askama::Template;

#[test]
fn escaped_history_keeps_the_latest_message_within_the_patch_bound() {
    let state = crate::tests::test_state(RuntimeConfig::development());
    for provider in crate::providers::ProviderKind::ALL {
        state
            .vault
            .put(crate::providers::ProviderConnection::with_key(
                provider,
                "test-key",
                "test-model",
            ))
            .unwrap();
    }
    let mut record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("record");
    record.messages = (0..8)
        .map(|_| ConversationMessage {
            role: MessageRole::User,
            text: "\"".repeat(32 * 1024),
            status: MessageStatus::Complete,
            error: None,
            request: None,
        })
        .collect();
    let view = ConversationDetailView::from_record(
        &record,
        ModelSources {
            vault: &state.vault,
            preferences: &state.preferences,
            models: &state.models_dev,
            environments: &state.environments,
            environment_snapshots: &state.environment_snapshots,
            projects: &[],
            documents: &[],
            presets: &[],
        },
        &[],
        None,
        false,
        &record.title,
        "",
    );
    assert!(view.omitted_messages > 0);
    assert_eq!(view.messages.last().expect("latest").index, 7);
    let mut patches = hypergraft::PatchSet::new();
    patches
        .children("conversation-detail", &view.contents())
        .expect("bounded patch");
    patches
        .encode_final(hypergraft::PatchStatus::Ok)
        .expect("bounded envelope");
    assert!(
        view.render()
            .expect("page")
            .contains("remain in local history and model context")
    );
}

#[test]
fn network_form_preserves_domains_without_a_live_preset_ceiling() {
    let state = crate::tests::test_state(RuntimeConfig::development());
    let mut record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("record");
    record.network =
        NetworkAccess::Restricted(vec!["example.com".to_owned(), "example.org".to_owned()]);
    let view = ConversationDetailView::from_record(
        &record,
        ModelSources {
            vault: &state.vault,
            preferences: &state.preferences,
            models: &state.models_dev,
            environments: &state.environments,
            environment_snapshots: &state.environment_snapshots,
            projects: &[],
            documents: &[],
            presets: &[],
        },
        &[],
        None,
        false,
        &record.title,
        "",
    );
    assert_eq!(
        NetworkAccess::parse_form("restricted", &view.network_domains)
            .expect("resubmitted domains"),
        record.network
    );
}

#[test]
fn dense_markup_uses_escaped_text_with_bounded_nodes() {
    let text = format!("{}<script>alert(1)</script>", "* item\n".repeat(8192));
    let html = reply_html(&text);
    assert!(html.matches('<').count() <= 64);
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
fn document_actions_keep_the_selected_revision_identity_after_a_correction() {
    let state = crate::tests::test_state(RuntimeConfig::development());
    let record = state.conversations.create("Documents".to_owned()).unwrap();
    let content = "# Tasks\n- [ ] First task\n";
    let document = state
        .documents
        .create_task_list_from_text(record.id, "Tasks".to_owned(), content.to_owned(), None)
        .unwrap();
    let hash = document.current().content_hash.as_str();
    let document = state
        .documents
        .revise(
            &document.id,
            1,
            "Tasks".to_owned(),
            "# Tasks\n- [ ] Replacement\n".to_owned(),
            None,
        )
        .unwrap();
    let view = PlanDocumentPage::from_document(&document, 1, content.to_owned(), "");
    assert_eq!(view.document_revision, 1);
    assert_eq!(view.current_revision, 2);
    assert!(view.action_href.contains(&format!(
        "task_document={}&task_revision=1&task_hash={hash}",
        document.id
    )));
    assert!(view.review_href.ends_with("?revision=1"));
    assert!(
        view.tasks[0]
            .run_href
            .contains(&format!("task_revision=1&task_hash={hash}&task_index=0"))
    );
}
