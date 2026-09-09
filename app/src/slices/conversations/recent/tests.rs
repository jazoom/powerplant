use super::{idle_status, status_dot};
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

use crate::slices::conversations::tests::{app, connected, document, navigation, test_state, text};

#[tokio::test]
async fn recent_projection_is_bounded_escaped_and_uses_the_canonical_catalogue_route() {
    let state = test_state();
    let token = connected(&state);
    for _ in 0..13 {
        state
            .conversations
            .create_saved(
                crate::conversations::ConversationId::generate().expect("id"),
                None,
                Some("<script>title</script>".to_owned()),
                None,
                vec![],
            )
            .expect("record");
    }
    let request = Request::builder()
        .uri("/conversations?index=true")
        .header(header::COOKIE, format!("powerplant_session={token}"))
        .header("Graft-Request", "patch")
        .header(header::ACCEPT, "text/vnd.hypergraft.patches+html")
        .body(Body::empty())
        .expect("request");
    let response = app(&state).oneshot(request).await.expect("projection");
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("target=\"recent-conversations\""));
    assert_eq!(body.matches("data-recent-conversation").count(), 12);
    assert!(!body.contains("<script>title</script>"));
    for request in [
        document("/conversations?index=true", &token),
        navigation("/conversations?index=true", &token),
    ] {
        let response = app(&state).oneshot(request).await.expect("canonical page");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            text(response)
                .await
                .contains("data-section=\"conversations\"")
        );
    }
}

/// Draft marks only untouched saved records. Idle records with a reply
/// stay ready, and attention states keep their review wording.
#[test]
fn draft_marks_only_untouched_records() {
    use crate::conversations::MessageStatus;
    assert_eq!(idle_status(None), "Draft");
    assert_eq!(idle_status(Some(MessageStatus::Complete)), "Ready");
    assert_eq!(idle_status(Some(MessageStatus::Pending)), "In progress");
    assert_eq!(idle_status(Some(MessageStatus::Failed)), "Response failed");
    assert_eq!(idle_status(Some(MessageStatus::Interrupted)), "Interrupted");
    assert_eq!(status_dot("Needs your review"), "attention");
    assert_eq!(status_dot("Needs command approval"), "attention");
    assert_eq!(status_dot("In progress"), "active");
    assert_eq!(status_dot("Draft"), "quiet");
    assert_eq!(status_dot("Ready"), "quiet");
    assert_eq!(status_dot("Completed"), "quiet");
    assert_eq!(status_dot("Cancelled"), "quiet");
}

#[tokio::test]
async fn sidebar_projection_carries_draft_status_dots_and_attention_count() {
    let state = test_state();
    let token = connected(&state);
    state
        .conversations
        .create_saved(
            crate::conversations::ConversationId::generate().expect("id"),
            None,
            Some("Quarterly planning".to_owned()),
            None,
            vec![],
        )
        .expect("record");
    let request = Request::builder()
        .uri("/conversations?index=true")
        .header(header::COOKIE, format!("powerplant_session={token}"))
        .header("Graft-Request", "patch")
        .header(header::ACCEPT, "text/vnd.hypergraft.patches+html")
        .body(Body::empty())
        .expect("request");
    let response = app(&state).oneshot(request).await.expect("projection");
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("target=\"recent-conversations\""));
    assert!(body.contains("target=\"attention-count\""));
    assert!(body.contains("Draft"));
    assert!(body.contains("state-dot"));
    let response = app(&state)
        .oneshot(document("/conversations", &token))
        .await
        .expect("catalogue");
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("recent-filter-input"));
    assert!(body.contains("All conversations"));
    assert!(body.contains("data-attention-count"));
    assert!(body.contains("Skip to main content"));
}

/// A live awaiting gate drives the sidebar badge and the review status
/// from the same server decision source, without a page reload.
#[tokio::test]
async fn sidebar_shows_live_attention_count_beside_needs_your_attention() {
    use crate::workflows::definition::{InputKey, OutputKey, StepKey};

    let state = test_state();
    let token = connected(&state);
    let project_dir = tempfile::tempdir().expect("work dir");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(project_dir.path())
            .status()
            .expect("git")
            .success()
    );
    std::fs::write(project_dir.path().join("notes.txt"), b"candidate\n").expect("source");
    let initial_capture = crate::workflows::artefacts::CandidateCapture::capture_host(
        project_dir.path(),
        &state.workflow_artefacts,
    )
    .expect("initial capture");
    std::fs::write(project_dir.path().join("notes.txt"), b"changed\n").expect("change");
    let produced_capture = crate::workflows::artefacts::CandidateCapture::capture_host(
        project_dir.path(),
        &state.workflow_artefacts,
    )
    .expect("changed capture");
    let conversation = state
        .conversations
        .create_saved(
            crate::conversations::ConversationId::generate().expect("id"),
            None,
            Some("Fix the timeout message".to_owned()),
            None,
            vec![],
        )
        .expect("record");
    let pinned = crate::workflows::pin_quick_task(
        crate::agents::AccessMode::ReadWrite,
        &[crate::agents::ToolId::List],
        "Fix the timeout message.",
        crate::tests::test_environment_id(),
    )
    .expect("quick task");
    let mut run = crate::workflows::WorkflowRun::create(
        crate::workflows::RunId::generate().expect("run"),
        1,
        crate::projects::ProjectId::generate().expect("project"),
        None,
        crate::workflows::RunKind::QuickTask,
        pinned.clone(),
        crate::tests::test_environment_set(&pinned.definition),
    );
    run.conversation_id = Some(conversation.id);
    let gate_run_id = run.id;
    let publish = |captured: &crate::workflows::artefacts::candidate::CandidateRevisionArtefact,
                   producer: crate::workflows::artefacts::ArtefactProducer,
                   inputs: Vec<crate::workflows::artefacts::ArtefactReference>| {
        let bytes = captured.manifest_bytes().expect("manifest");
        let object = state.workflow_artefacts.publish(&bytes).expect("publish");
        crate::workflows::artefacts::ArtefactRecord {
            id: crate::workflows::ArtefactId::generate().expect("artefact"),
            kind: crate::workflows::definition::ArtefactKind::CandidateRevision,
            artefact_hash: crate::workflows::artefacts::artefact_hash_for(
                crate::workflows::definition::ArtefactKind::CandidateRevision,
                captured.format_version,
                &bytes,
            ),
            object_hash: object,
            payload_bytes: bytes.len() as u64,
            created_at_ms: 1,
            provenance: crate::workflows::artefacts::ArtefactProvenance {
                run_id: gate_run_id,
                producer,
                inputs,
            },
            summary: crate::workflows::artefacts::ArtefactSummary::Candidate {
                candidate: captured.candidate_hash,
                entries: captured.entries.len() as u64,
                bytes: 0,
                disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
            },
        }
    };
    let initial = publish(
        &initial_capture,
        crate::workflows::artefacts::ArtefactProducer::RunSourceCapture,
        Vec::new(),
    );
    let initial_ref = crate::workflows::artefacts::ArtefactReference {
        id: initial.id,
        kind: initial.kind,
        artefact_hash: initial.artefact_hash,
    };
    run.record_initial_candidate(initial).expect("initial");
    let work = StepKey::parse("work").expect("work");
    let attempt = crate::workflows::AttemptId::generate().expect("attempt");
    run.start_attempt(
        attempt,
        vec![crate::workflows::run::AttemptArtefactInput {
            key: InputKey::parse("candidate").expect("input"),
            artefact: initial_ref.clone(),
        }],
        crate::tests::test_agent_capabilities(),
        crate::workflows::run::AttemptSandboxRecord {
            kind: crate::workflows::run::AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: run
                .environments
                .steps
                .iter()
                .find(|binding| binding.step == work)
                .expect("work environment")
                .snapshot_digest
                .clone(),
        },
        2,
    )
    .expect("start");
    let produced = publish(
        &produced_capture,
        crate::workflows::artefacts::ArtefactProducer::StepAttempt {
            attempt_id: attempt,
            step: work.clone(),
            output: Some(OutputKey::parse("candidate").expect("output")),
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
        vec![initial_ref.clone()],
    );
    let produced_ref = crate::workflows::artefacts::ArtefactReference {
        id: produced.id,
        kind: produced.kind,
        artefact_hash: produced.artefact_hash,
    };
    run.record_attempt_outputs(
        attempt,
        vec![produced],
        vec![crate::workflows::run::AttemptArtefactOutput {
            key: OutputKey::parse("candidate").expect("output"),
            artefact: produced_ref.clone(),
        }],
        Some(produced_ref.clone()),
        crate::workflows::run::ObservedCandidate::Exact {
            artefact: produced_ref.clone(),
        },
    )
    .expect("outputs");
    run.record_cleanup(
        attempt,
        crate::workflows::run::AttemptCleanupRecord::Complete,
    )
    .expect("cleanup");
    run.complete_attempt(attempt, 3).expect("complete work");
    run.open_gate(
        crate::workflows::GateId::generate().expect("gate"),
        produced_ref,
        initial_ref,
        4,
    )
    .expect("gate");
    state.workflow_runs.create(run).expect("store run");
    let request = Request::builder()
        .uri("/conversations?index=true")
        .header(header::COOKIE, format!("powerplant_session={token}"))
        .header("Graft-Request", "patch")
        .header(header::ACCEPT, "text/vnd.hypergraft.patches+html")
        .body(Body::empty())
        .expect("request");
    let response = app(&state).oneshot(request).await.expect("projection");
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("target=\"attention-count\""));
    assert!(body.contains(">1</template>"));
    assert!(body.contains("Needs your review"));
    assert!(body.contains("state-dot attention"));
    let response = app(&state)
        .oneshot(document("/conversations", &token))
        .await
        .expect("catalogue");
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains(">1</span"));
    assert!(body.contains("Needs your review"));
}
