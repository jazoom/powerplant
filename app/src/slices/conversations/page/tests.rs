use super::*;
use crate::config::RuntimeConfig;
use askama::Template;

#[test]
fn escaped_history_keeps_the_latest_message_within_the_patch_bound() {
    let state = crate::tests::test_state(RuntimeConfig::development());
    let mut record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("record");
    record.messages = (0..8)
        .map(|_| ConversationMessage {
            role: MessageRole::User,
            text: "\"".repeat(32 * 1024),
            status: MessageStatus::Complete,
            request: None,
        })
        .collect();
    let view = ConversationDetailView::from_record(
        &record,
        ModelSources {
            vault: &state.vault,
            models: &state.models_dev,
            projects: &[],
            documents: &[],
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
fn network_form_preserves_domains_and_shows_the_narrower_preset_ceiling() {
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
            models: &state.models_dev,
            projects: &[],
            documents: &[],
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
    let effective = NetworkAccess::Restricted(vec![
        "api.example.com".to_owned(),
        "api.example.org".to_owned(),
    ]);
    let summary = format_network_summary(&record.network, &effective);
    assert!(summary.contains(
        "Effective with preset ceiling: Restricted domains: api.example.com, api.example.org"
    ));
}

#[test]
fn dense_markup_uses_escaped_text_with_bounded_nodes() {
    let text = format!("{}<script>alert(1)</script>", "* item\n".repeat(8192));
    let html = reply_html(&text);
    assert!(html.matches('<').count() <= 64);
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
}
