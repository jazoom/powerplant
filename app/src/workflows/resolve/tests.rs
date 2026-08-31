use super::{ResolveEnvironmentError, resolve_environments};
use crate::agents::ToolId;
use crate::environments::snapshot::tests_support::sample_snapshot;
use crate::environments::{
    EnvironmentCatalogue, EnvironmentDraft, EnvironmentSnapshotRepository, SnapshotAvailability,
};
use crate::workflows::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, CandidateAuthority, OutputKey, OutputKind,
    RequiredOutput, RoleDefinition, RoleKey, StepAction, StepDefinition, StepEnvironment, StepKey,
    SuccessTransition, WorkflowDefinition, candidate_revision_output, initial_candidate_input,
    test_environment_id,
};

fn draft(name: &str) -> EnvironmentDraft {
    EnvironmentDraft {
        name: name.to_owned(),
        oci_image: "alpine/git".to_owned(),
        setup_script: String::new(),
    }
}

fn definition(default: crate::environments::EnvironmentId) -> WorkflowDefinition {
    let role = RoleDefinition::new(
        RoleKey::parse("agent").expect("role"),
        "Coding agent".to_owned(),
        String::new(),
        String::new(),
    )
    .expect("role");
    let authority = AgentAuthority::new(vec![ToolId::List], Vec::new()).expect("authority");
    WorkflowDefinition::from_parts(
        "One step".to_owned(),
        default,
        vec![role],
        vec![StepDefinition {
            key: StepKey::parse("work").expect("step"),
            name: "Work on task".to_owned(),
            inputs: vec![initial_candidate_input()],
            action: StepAction::Agent(AgentStep {
                environment: StepEnvironment::WorkflowDefault,
                role: RoleKey::parse("agent").expect("role"),
                candidate_authority: CandidateAuthority::Edit,
                authority,
                required_outputs: vec![
                    RequiredOutput {
                        key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                        kind: OutputKind::AssistantReply,
                    },
                    candidate_revision_output(),
                ],
            }),
            on_success: SuccessTransition::CompleteRun,
        }],
    )
    .expect("definition")
}

#[tokio::test]
async fn missing_environments_are_rejected() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let snapshots = EnvironmentSnapshotRepository::in_memory();
    let error = resolve_environments(&definition(test_environment_id()), &catalogue, &snapshots)
        .await
        .err();
    assert_eq!(error, Some(ResolveEnvironmentError::Missing));
}

#[tokio::test]
async fn unready_environments_are_rejected() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let snapshots = EnvironmentSnapshotRepository::in_memory();
    let (record, _) = catalogue.create(draft("Alpine Git")).expect("create");
    let error = resolve_environments(&definition(record.id), &catalogue, &snapshots)
        .await
        .err();
    assert_eq!(error, Some(ResolveEnvironmentError::NotReady));
}

#[tokio::test]
async fn unavailable_snapshots_are_rejected() {
    let catalogue = EnvironmentCatalogue::in_memory();
    let snapshots = EnvironmentSnapshotRepository::in_memory();
    let (record, preparation) = catalogue.create(draft("Alpine Git")).expect("create");
    catalogue.claim_oldest_queued().expect("claim");
    let snapshot = sample_snapshot(preparation.id);
    snapshots.mark(snapshot.artifact_key.clone(), SnapshotAvailability::Corrupt);
    catalogue
        .finish_ready(&preparation.id, snapshot, preparation.log)
        .expect("ready");
    let error = resolve_environments(&definition(record.id), &catalogue, &snapshots)
        .await
        .err();
    assert_eq!(error, Some(ResolveEnvironmentError::Unavailable));
}
