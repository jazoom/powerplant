use super::{ONE_AGENT_V1, SeedKey, WorkflowSeed, one_agent_definition};
use crate::workflows::catalogue::WorkflowCatalogue;
use crate::workflows::definition::{StepAction, WorkflowDefinition};

fn named(name: &str) -> WorkflowDefinition {
    let mut definition = one_agent_definition();
    definition = WorkflowDefinition::from_parts(
        name.to_owned(),
        definition.roles().to_vec(),
        definition.first_step().clone(),
        definition.steps().to_vec(),
    )
    .expect("named");
    definition
}

#[test]
fn first_open_seeds_one_agent_once() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let first = WorkflowCatalogue::open(path.clone()).expect("open");
    let records = first.list();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].definition.name(), "One agent");
    assert_eq!(first.applied_seed_count(), 1);
    let second = WorkflowCatalogue::open(path).expect("reopen");
    assert_eq!(second.list().len(), 1);
    assert_eq!(second.list()[0].id, records[0].id);
}

#[test]
fn restart_preserves_an_edited_seeded_workflow() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let catalogue = WorkflowCatalogue::open(path.clone()).expect("open");
    let seeded = catalogue.list().into_iter().next().expect("seed");
    catalogue
        .update(&seeded.id, seeded.revision, named("Edited agent"))
        .expect("edit");
    let reopened = WorkflowCatalogue::open(path).expect("reopen");
    let loaded = reopened.list();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, seeded.id);
    assert_eq!(loaded[0].definition.name(), "Edited agent");
}

#[test]
fn restart_does_not_restore_a_deleted_seeded_workflow() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let catalogue = WorkflowCatalogue::open(path.clone()).expect("open");
    let seeded = catalogue.list().into_iter().next().expect("seed");
    catalogue
        .delete(&seeded.id, seeded.revision)
        .expect("delete");
    assert!(catalogue.list().is_empty());
    assert!(catalogue.retired_ids().contains(&seeded.id));
    assert_eq!(catalogue.applied_seed_count(), 1);
    let reopened = WorkflowCatalogue::open(path).expect("reopen");
    assert!(reopened.list().is_empty());
    assert!(reopened.retired_ids().contains(&seeded.id));
    assert_eq!(reopened.applied_seed_count(), 1);
}

#[test]
fn a_present_seed_key_is_not_reapplied_from_code() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let catalogue = WorkflowCatalogue::open_with_seeds(
        path.clone(),
        &[WorkflowSeed {
            key: SeedKey::parse(ONE_AGENT_V1).expect("key"),
            definition: named("Custom"),
        }],
    )
    .expect("open");
    assert_eq!(catalogue.list()[0].definition.name(), "Custom");
    let reopened = WorkflowCatalogue::open(path).expect("production reopen");
    assert_eq!(reopened.list().len(), 1);
    assert_eq!(reopened.list()[0].definition.name(), "Custom");
}

#[test]
fn scheduler_inputs_contain_no_seed_provenance() {
    let definition = one_agent_definition();
    let bytes = serde_json::to_vec(&definition.to_file()).expect("json");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(!text.contains(ONE_AGENT_V1));
    assert!(!text.contains("built_in"));
    assert!(!text.contains("built-in"));
    assert!(matches!(definition.steps()[0].action, StepAction::Agent(_)));
}
