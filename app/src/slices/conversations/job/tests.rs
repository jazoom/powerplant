use crate::{
    conversations::{ConversationMessage, ConversationRecord, MessageRole, MessageStatus},
    sessions::JobId,
};

use super::history;
use crate::{
    config::RuntimeConfig,
    providers::{
        ChatBackend, ModelSelection, ProviderConnection, ProviderError, ProviderKind,
        tests::ScriptedBackend,
    },
    sessions::{JobStatus, generate_session_token},
};
use std::sync::Arc;

#[tokio::test]
async fn bounded_partial_reply_settles_and_observation_restores_commands() {
    let mut state = crate::tests::test_state(RuntimeConfig::development());
    let backend = ScriptedBackend::chunks([
        Ok("Partial reply".to_owned()),
        Ok("x".repeat(crate::conversations::MAXIMUM_REPLY_BYTES)),
    ]);
    state.chat = Arc::new(ChatBackend::Scripted(backend.clone()));
    let token = generate_session_token().expect("session");
    state.sessions.insert(token.id());
    let connection = ProviderConnection::with_key(ProviderKind::Xai, "test-key", "grok-4.6");
    state.vault.put(connection.clone()).expect("provider");
    let record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let job = state
        .sessions
        .begin_conversation_job(&token.id(), record.id, 1)
        .expect("job");
    let selection =
        ModelSelection::new(connection.kind, connection.model.clone(), None).expect("selection");
    let record = state
        .conversations
        .begin_message(
            &record.id,
            record.revision,
            selection,
            job.id(),
            "Question".to_owned(),
        )
        .expect("begin");
    super::run(
        state.clone(),
        token.id(),
        record.id,
        record.clone(),
        connection,
        job.clone(),
    )
    .await;
    let saved = state.conversations.get(&record.id).expect("saved");
    assert_eq!(saved.messages[1].text, "Partial reply");
    assert_eq!(saved.messages[1].status, MessageStatus::Failed);
    assert!(saved.active_job.is_none());
    assert!(!state.sessions.busy(&token.id()));
    assert!(backend.last_tools().is_empty());
    assert_eq!(backend.last_preamble().as_deref(), Some(""));
    let frame = super::final_frame(&state, &record.id, token.id(), &job, job.latest_seq());
    let body = String::from_utf8(frame.into_bytes()).expect("frame");
    assert!(body.contains("target=\"conversation-detail\""));
    assert!(body.contains(&format!("name=\"revision\" value=\"{}\"", saved.revision)));
    assert!(!body.contains("data-observe-active"));
}

#[tokio::test]
async fn cancellation_and_stale_settlement_cannot_release_another_command() {
    let state = crate::tests::test_state(RuntimeConfig::development());
    let token = generate_session_token().expect("session");
    let other = generate_session_token().expect("other session");
    state.sessions.insert(token.id());
    state.sessions.insert(other.id());
    let record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let another = state
        .conversations
        .create("Another".to_owned())
        .expect("conversation");
    let job = state
        .sessions
        .begin_conversation_job(&token.id(), record.id, 1)
        .expect("job");
    assert!(
        state
            .sessions
            .begin_conversation_job(&token.id(), another.id, 1)
            .is_err()
    );
    assert!(
        state
            .sessions
            .begin_conversation_job(&other.id(), record.id, 1)
            .is_err()
    );
    assert!(
        !state
            .sessions
            .finish_conversation_job(&other.id(), record.id, job.id())
    );
    let connection = ProviderConnection::with_key(ProviderKind::Xai, "test-key", "grok-4.6");
    let selection =
        ModelSelection::new(connection.kind, connection.model.clone(), None).expect("selection");
    let record = state
        .conversations
        .begin_message(
            &record.id,
            record.revision,
            selection,
            job.id(),
            "Question".to_owned(),
        )
        .expect("begin");
    job.request_cancel();
    super::run(
        state.clone(),
        token.id(),
        record.id,
        record.clone(),
        connection,
        job.clone(),
    )
    .await;
    assert_eq!(job.snapshot().status, JobStatus::Cancelled);
    assert_eq!(
        state
            .conversations
            .get(&record.id)
            .expect("record")
            .messages[1]
            .status,
        MessageStatus::Interrupted
    );
    let next = state
        .sessions
        .begin_conversation_job(&token.id(), another.id, 1)
        .expect("next job");
    assert!(
        !state
            .sessions
            .finish_conversation_job(&token.id(), record.id, job.id())
    );
    assert!(state.sessions.busy(&token.id()));
    state.sessions.remove(&token.id());
    assert!(next.cancel_requested());
}

#[tokio::test]
async fn provider_failure_retains_partial_output() {
    let mut state = crate::tests::test_state(RuntimeConfig::development());
    state.chat = Arc::new(ChatBackend::Scripted(ScriptedBackend::chunks([
        Ok("Partial".to_owned()),
        Err(ProviderError::Unreachable),
    ])));
    let token = generate_session_token().expect("session");
    state.sessions.insert(token.id());
    let record = state
        .conversations
        .create("Discussion".to_owned())
        .expect("conversation");
    let job = state
        .sessions
        .begin_conversation_job(&token.id(), record.id, 1)
        .expect("job");
    let connection = ProviderConnection::with_key(ProviderKind::Xai, "test-key", "grok-4.6");
    let selection =
        ModelSelection::new(connection.kind, connection.model.clone(), None).expect("model");
    let record = state
        .conversations
        .begin_message(
            &record.id,
            record.revision,
            selection,
            job.id(),
            "Question".to_owned(),
        )
        .expect("begin");
    super::run(
        state.clone(),
        token.id(),
        record.id,
        record.clone(),
        connection,
        job,
    )
    .await;
    let record = state.conversations.get(&record.id).expect("record");
    assert_eq!(record.messages[1].text, "Partial");
    assert_eq!(record.messages[1].status, MessageStatus::Failed);
    assert!(record.active_job.is_none());
}

#[test]
fn pending_assistant_output_stays_out_of_the_next_request_history() {
    let request = JobId::generate().expect("request");
    let record = ConversationRecord {
        id: crate::conversations::ConversationId::generate().expect("conversation"),
        revision: 1,
        title: "Discussion".to_owned(),
        projects: Vec::new(),
        model: None,
        messages: vec![
            ConversationMessage {
                role: MessageRole::User,
                text: "First question".to_owned(),
                status: MessageStatus::Complete,
                request: None,
            },
            ConversationMessage {
                role: MessageRole::Assistant,
                text: "First reply".to_owned(),
                status: MessageStatus::Complete,
                request: Some(JobId::generate().expect("previous request")),
            },
            ConversationMessage {
                role: MessageRole::User,
                text: "Second question".to_owned(),
                status: MessageStatus::Complete,
                request: None,
            },
            ConversationMessage {
                role: MessageRole::Assistant,
                text: String::new(),
                status: MessageStatus::Pending,
                request: Some(request),
            },
        ],
        active_job: Some(request),
        created_at_ms: 0,
        updated_at_ms: 0,
    };

    let history = history(&record);
    assert_eq!(history.len(), 3);
    assert_eq!(history.last().expect("last turn").text, "Second question");
}
