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
fn dense_markup_uses_escaped_text_with_bounded_nodes() {
    let text = format!("{}<script>alert(1)</script>", "* item\n".repeat(8192));
    let html = reply_html(&text);
    assert!(html.matches('<').count() <= 64);
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
}
