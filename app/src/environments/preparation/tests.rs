use super::{FailureCategory, PreparationPhase, PreparationRecord, PreparationState};
use crate::environments::id::{EnvironmentId, PreparationId};
use crate::environments::recipe::{EnvironmentDraft, EnvironmentRecipe};

fn queued() -> PreparationRecord {
    let recipe = EnvironmentRecipe::from_draft(&EnvironmentDraft {
        name: "Env".to_owned(),
        oci_image: "alpine/git".to_owned(),
        setup_script: String::new(),
    })
    .expect("recipe")
    .1;
    PreparationRecord::queued(
        PreparationId::generate().expect("prep"),
        EnvironmentId::generate().expect("env"),
        1,
        1,
        recipe.version(),
        10,
    )
}

#[test]
fn queued_and_ready_combinations_are_closed() {
    let mut record = queued();
    assert!(record.validate_combination());
    record.state = PreparationState::Ready;
    record.phase = PreparationPhase::Finished;
    assert!(!record.validate_combination());
    record.started_at_ms = Some(11);
    record.finished_at_ms = Some(12);
    assert!(!record.validate_combination());
}

#[test]
fn failed_states_require_a_failure_and_reject_snapshots() {
    let mut record = queued();
    record.state = PreparationState::Failed;
    record.phase = PreparationPhase::RunningSetup;
    record.started_at_ms = Some(11);
    record.finished_at_ms = Some(12);
    record.failure = Some(super::PreparationFailure::new(FailureCategory::SetupExit));
    assert!(record.validate_combination());
    record.snapshot =
        Some(crate::environments::snapshot::tests_support::sample_snapshot(record.id));
    assert!(!record.validate_combination());
}

#[test]
fn cancelled_and_superseded_reject_failures() {
    let mut record = queued();
    record.state = PreparationState::Cancelled;
    record.finished_at_ms = Some(12);
    assert!(record.validate_combination());
    record.failure = Some(super::PreparationFailure::new(
        FailureCategory::EnvironmentDeleted,
    ));
    assert!(!record.validate_combination());
}

#[test]
fn phase_labels_are_stable() {
    assert_eq!(PreparationPhase::CreatingGuest.as_str(), "creating-guest");
    assert_eq!(
        PreparationPhase::parse("verifying-snapshot"),
        Some(PreparationPhase::VerifyingSnapshot)
    );
    assert!(PreparationPhase::CreatingGuest < PreparationPhase::RunningSetup);
}
