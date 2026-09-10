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
    let mut record = state.conversations.create("Documents".to_owned()).unwrap();
    let content = "# Tasks\n- [ ] First task\n";
    let document = state
        .documents
        .create_task_list_from_text(&record, "Tasks".to_owned(), content.to_owned(), None)
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
    let view = PlanDocumentPage::from_document(&document, 1, content.to_owned(), 0, "");
    assert_eq!(view.document_revision, 1);
    assert_eq!(view.current_revision, 2);
    assert!(view.action_href.contains(&format!(
        "task_document={}&task_revision=1&task_hash={hash}",
        document.id
    )));
    assert!(view.review_href.ends_with("?revision=1"));
    record.messages.push(ConversationMessage {
        role: MessageRole::User,
        text: "A later message".to_owned(),
        status: MessageStatus::Complete,
        error: None,
        request: None,
    });
    let token = super::super::tests::connected(&state);
    let session = super::super::tests::session_id(&token);
    let transcript = super::super::detail_view(&state, session, &record, &record.title, "");
    assert!(transcript.messages[0].html.contains("First task"));
    assert!(!transcript.messages[0].html.contains("Replacement"));
    assert_eq!(transcript.messages[1].index, 0);
    assert!(
        view.tasks[0]
            .run_href
            .contains(&format!("task_revision=1&task_hash={hash}&task_index=0"))
    );
}

#[test]
fn candidate_review_escapes_untrusted_file_contents() {
    let state = crate::tests::test_state(RuntimeConfig::development());
    let record = state.conversations.create("Discussion".to_owned()).unwrap();
    let title = record.title.clone();
    let gate = super::PendingCodeGateView {
        run_id: "run".to_owned(),
        gate_id: "gate".to_owned(),
        revision: "1".to_owned(),
        candidate: "abc".to_owned(),
        diff_base: "def".to_owned(),
        diff_href: "/runs/run/gates/gate".to_owned(),
        review_href: "/conversations/candidate-review?run=run".to_owned(),
        ordinary: true,
        application_destination: "/tmp/test".to_owned(),
        can_request_revision: false,
        quick_task: true,
        exclusions: Vec::new(),
        total_changes: 1,
        changes_truncated: false,
        changes: vec![super::CandidateChangeView {
            path: "<script>alert(1)</script>".to_owned(),
            name: "<script>alert(1)</script>".to_owned(),
            directory: "project".to_owned(),
            status: "Added",
            preview: "+<img src=x onerror=alert(1)>\n".to_owned(),
            additions: 1,
            removals: 0,
            has_counts: true,
        }],
    };
    let view = ConversationDetailView::from_record_with_gate(
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
        &title,
        "",
        Some(gate),
        None,
        Vec::new(),
        None,
        Vec::new(),
    );
    let html = view.render().expect("page");
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(!html.contains("<img src=x"));
    assert!(html.contains("&#60;script&#62;"));
    assert!(html.contains("&#60;img"));
}

#[derive(askama::Template)]
#[template(path = "conversations/templates/workflow_progress.html")]
struct ProgressHarness {
    run: WorkflowProgressView,
}

fn partial_progress_view() -> WorkflowProgressView {
    WorkflowProgressView {
        run_href: "/runs/aaa".to_owned(),
        name: "Quick task".to_owned(),
        state: "Active",
        current_step: "Apply changes".to_owned(),
        result: "Worker activity stays in the run record.",
        task_progress: String::new(),
        loop_id: String::new(),
        command_token: String::new(),
        can_pause: false,
        can_continue: false,
        can_retry: false,
        can_stop: false,
        pause_requested: false,
        awaiting_gate: false,
        conversation_id: "ccc".to_owned(),
        apply_run_id: "aaa".to_owned(),
        apply_attempt_id: "bbb".to_owned(),
        apply_state: "recovered",
        apply_outcomes: vec![ApplyOutcomeView {
            directory: "fieldnotes".to_owned(),
            path: "/tmp/fieldnotes".to_owned(),
            outcome: "Applied",
        }],
        apply_resolve_href: "/runs/aaa/attempts/bbb/changes".to_owned(),
        apply_partial: true,
        apply_uncertain: false,
        apply_complete: false,
        settlement_eligible: true,
        run_terminal: false,
    }
}

// Recovery controls bind the displayed run, attempt and outcome state.
#[test]
fn partial_progress_links_the_exact_attempt_and_settlement_identity() {
    use askama::Template;
    let html = ProgressHarness {
        run: partial_progress_view(),
    }
    .render()
    .expect("progress");
    assert!(html.contains("/runs/aaa/attempts/bbb/changes"));
    assert!(html.contains("/conversations/ccc/runs/aaa/settle-partial"));
    assert!(html.contains("name=\"attempt\""));
    assert!(html.contains("value=\"bbb\""));
    assert!(html.contains("value=\"recovered\""));
}

#[test]
fn failed_loop_retains_the_exact_child_application_evidence() {
    use crate::workflows::apply::{ApplyTransaction, ApplyTransactionState};
    use crate::workflows::task_loop::{TaskLoopState, TaskOutcome};
    let state = crate::tests::test_state(RuntimeConfig::development());
    let token = super::super::tests::connected(&state);
    super::super::tests::awaiting_gate(&state);
    let run = state.workflow_runs.active_runs().remove(0);
    let conversation = state
        .conversations
        .get(&run.conversation_id.unwrap())
        .unwrap();
    let mut parent = crate::workflows::task_loop::tests::loop_record();
    parent.conversation_id = conversation.id;
    parent.state = TaskLoopState::Failed;
    parent.tasks[0].child_id = Some(run.id);
    parent.tasks[0].outcome = TaskOutcome::Failed;
    let attempt = run.attempts[0].id;
    state
        .workflow_runs
        .mutate(&run.id, |child| {
            child.parent_loop = Some(parent.id);
            child.state = crate::workflows::run::RunState::Failed;
            child.attempts[0].apply_transaction = Some(ApplyTransaction {
                state: ApplyTransactionState::Recovered,
                roots: Vec::new(),
                baseline: child.gates[0].diff_base.clone(),
                candidate: child.gates[0].candidate.clone(),
                approval: child.gates[0].candidate.clone(),
            });
            Ok(())
        })
        .unwrap();
    state.task_loops.create(parent).unwrap();
    let view = super::super::detail_view(
        &state,
        super::super::tests::session_id(&token),
        &conversation,
        &conversation.title,
        "",
    );
    let progress = view.saved().unwrap().workflow_progress.as_ref().unwrap();
    assert!(progress.apply_partial);
    assert_eq!(progress.apply_run_id, run.id.as_hex());
    assert_eq!(progress.apply_attempt_id, attempt.as_hex());
    assert_eq!(
        progress.apply_resolve_href,
        format!("/runs/{}/attempts/{attempt}/changes", run.id)
    );
}
