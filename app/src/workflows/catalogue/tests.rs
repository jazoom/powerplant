use super::{
    CatalogueError, ResolveWorkflowError, SELECTION_TOKEN_BYTES, WorkflowCatalogue,
    WorkflowSelection, definition_fits_agent,
};
use crate::agents::{AccessMode, ToolId};
use crate::workflows::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, CandidateAuthority, OutputKey, OutputKind,
    RequiredOutput, RoleDefinition, RoleKey, StepAction, StepDefinition, StepEnvironment, StepKey,
    SuccessTransition, SystemCommandId, SystemCommandStep, WorkflowDefinition,
    candidate_revision_output, initial_candidate_input, test_environment_id, test_named_definition,
};
use crate::workflows::id::WorkflowId;

fn named(name: &str) -> WorkflowDefinition {
    test_named_definition(name)
}

fn write_catalogue(path: &std::path::Path, json: &str) {
    std::fs::write(path, json).expect("write");
}

#[test]
fn create_assigns_a_random_identifier_and_first_revision() {
    let catalogue = WorkflowCatalogue::in_memory();
    let record = catalogue.create(named("Planner")).expect("create");
    assert_eq!(record.revision, 1);
    assert_eq!(record.created_at_ms, record.updated_at_ms);
    assert_eq!(record.definition_version, record.definition.version());
    assert_eq!(catalogue.list().len(), 1);
    assert_eq!(catalogue.get(&record.id).expect("get").id, record.id);
}

#[test]
fn create_rejects_duplicate_names_without_case_differences() {
    let catalogue = WorkflowCatalogue::in_memory();
    catalogue.create(named("Planner")).expect("create");
    assert_eq!(
        catalogue.create(named("planner")).err(),
        Some(CatalogueError::DuplicateName)
    );
}

#[test]
fn identical_updates_do_not_change_revision_or_version() {
    let catalogue = WorkflowCatalogue::in_memory();
    let created = catalogue.create(named("Planner")).expect("create");
    let updated = catalogue
        .update(&created.id, created.revision, named("Planner"))
        .expect("update");
    assert_eq!(updated.revision, created.revision);
    assert_eq!(updated.definition_version, created.definition_version);
    assert_eq!(updated.updated_at_ms, created.updated_at_ms);
}

#[test]
fn a_material_update_increments_revision_and_version() {
    let catalogue = WorkflowCatalogue::in_memory();
    let created = catalogue.create(named("Planner")).expect("create");
    let updated = catalogue
        .update(&created.id, created.revision, named("Reviewer"))
        .expect("update");
    assert_eq!(updated.revision, created.revision + 1);
    assert_ne!(updated.definition_version, created.definition_version);
    assert_eq!(updated.definition.name(), "Reviewer");
}

#[test]
fn stale_updates_and_deletes_conflict() {
    let catalogue = WorkflowCatalogue::in_memory();
    let created = catalogue.create(named("Planner")).expect("create");
    catalogue
        .update(&created.id, created.revision, named("Reviewer"))
        .expect("update");
    assert_eq!(
        catalogue
            .update(&created.id, created.revision, named("Later"))
            .err(),
        Some(CatalogueError::Conflict)
    );
    assert_eq!(
        catalogue.delete(&created.id, created.revision).err(),
        Some(CatalogueError::Conflict)
    );
}

#[test]
fn delete_retires_the_identifier_and_keeps_no_active_record() {
    let catalogue = WorkflowCatalogue::in_memory();
    let created = catalogue.create(named("Planner")).expect("create");
    catalogue
        .delete(&created.id, created.revision)
        .expect("delete");
    assert!(catalogue.get(&created.id).is_none());
    assert!(catalogue.retired_ids().contains(&created.id));
    let later = catalogue.create(named("Planner")).expect("recreate");
    assert_ne!(later.id, created.id);
}

#[test]
fn version_one_definition_hash_survives_reopen() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let id;
    let version;
    {
        let catalogue = WorkflowCatalogue::open_with_seeds(path.clone(), &[]).expect("open");
        let record = catalogue.create(named("Planner")).expect("create");
        id = record.id;
        version = record.definition_version;
    }
    let bytes = std::fs::read(&path).expect("read");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert!(
        json["workflows"][0]["definition"]
            .get("first-step")
            .is_none()
    );
    assert_eq!(json["workflows"][0]["definition-version"], version.as_hex());

    let catalogue = WorkflowCatalogue::open_with_seeds(path, &[]).expect("reopen");
    let loaded = catalogue.get(&id).expect("loaded");
    assert_eq!(loaded.definition.name(), "Planner");
    assert_eq!(loaded.definition_version, version);
}

#[test]
fn corrupt_files_fail_open() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    write_catalogue(&path, "{");
    assert_eq!(
        WorkflowCatalogue::open_with_seeds(path, &[]).err(),
        Some(CatalogueError::Corrupt)
    );
}

#[test]
fn overlapping_active_and_retired_identifiers_are_rejected() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let id = WorkflowId::generate().expect("id");
    let definition = named("Planner");
    let json = serde_json::json!({
        "file-version": 1,
        "applied-seeds": [],
        "retired-workflow-ids": [id.as_hex()],
        "workflows": [{
            "id": id.as_hex(),
            "revision": 1,
            "definition-version": definition.version().as_hex(),
            "definition": definition.to_file(),
            "created-at-ms": 1,
            "updated-at-ms": 1
        }]
    });
    write_catalogue(&path, &json.to_string());
    assert_eq!(
        WorkflowCatalogue::open_with_seeds(path, &[]).err(),
        Some(CatalogueError::Corrupt)
    );
}

#[test]
fn invalid_definition_digests_are_rejected() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let id = WorkflowId::generate().expect("id");
    let definition = named("Planner");
    let json = serde_json::json!({
        "file-version": 1,
        "applied-seeds": [],
        "retired-workflow-ids": [],
        "workflows": [{
            "id": id.as_hex(),
            "revision": 1,
            "definition-version": "0".repeat(64),
            "definition": definition.to_file(),
            "created-at-ms": 1,
            "updated-at-ms": 1
        }]
    });
    write_catalogue(&path, &json.to_string());
    assert_eq!(
        WorkflowCatalogue::open_with_seeds(path, &[]).err(),
        Some(CatalogueError::Corrupt)
    );
}

#[test]
fn seed_ledger_entries_must_name_a_known_identifier() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let json = serde_json::json!({
        "file-version": 1,
        "applied-seeds": [{
            "key": "one-agent-v1",
            "workflow-id": WorkflowId::generate().expect("id").as_hex()
        }],
        "retired-workflow-ids": [],
        "workflows": []
    });
    write_catalogue(&path, &json.to_string());
    assert_eq!(
        WorkflowCatalogue::open_with_seeds(path, &[]).err(),
        Some(CatalogueError::Corrupt)
    );
}

#[test]
fn malformed_seed_ledger_identifiers_are_rejected() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let json = serde_json::json!({
        "file-version": 1,
        "applied-seeds": [{
            "key": "one-agent-v1",
            "workflow-id": "not-an-id"
        }],
        "retired-workflow-ids": [],
        "workflows": []
    });
    write_catalogue(&path, &json.to_string());
    assert_eq!(
        WorkflowCatalogue::open_with_seeds(path, &[]).err(),
        Some(CatalogueError::Corrupt)
    );
}

#[test]
fn selection_tokens_reject_malformed_syntax() {
    let id = WorkflowId::generate().expect("id");
    let version = named("Planner").version();
    let token = WorkflowSelection {
        workflow_id: id,
        definition_version: version,
    }
    .as_token();
    assert_eq!(token.len(), SELECTION_TOKEN_BYTES);
    assert_eq!(
        WorkflowSelection::parse(&token),
        Some(WorkflowSelection {
            workflow_id: id,
            definition_version: version
        })
    );
    assert!(WorkflowSelection::parse(&token.to_ascii_uppercase()).is_none());
    assert!(WorkflowSelection::parse(&token.replace(':', "/")).is_none());
    assert!(WorkflowSelection::parse(&format!(" {token}")).is_none());
    assert!(WorkflowSelection::parse(&format!("{token}a")).is_none());
    assert!(WorkflowSelection::parse("").is_none());
    assert!(WorkflowSelection::parse(&id.as_hex()).is_none());
}

#[test]
fn resolve_returns_missing_changed_and_pinned_copies() {
    let catalogue = WorkflowCatalogue::in_memory();
    let created = catalogue.create(named("Planner")).expect("create");
    let selection = WorkflowSelection {
        workflow_id: created.id,
        definition_version: created.definition_version,
    };
    let resolved = catalogue.resolve(&selection).expect("resolve");
    assert_eq!(resolved.pinned.workflow_id, Some(created.id));
    assert_eq!(resolved.pinned.version, created.definition_version);
    assert_eq!(resolved.record_revision, 1);

    let updated = catalogue
        .update(&created.id, created.revision, named("Reviewer"))
        .expect("update");
    assert_eq!(
        catalogue.resolve(&selection).err(),
        Some(ResolveWorkflowError::Changed)
    );
    let current = WorkflowSelection {
        workflow_id: created.id,
        definition_version: updated.definition_version,
    };
    assert!(catalogue.resolve(&current).is_ok());

    catalogue
        .delete(&created.id, updated.revision)
        .expect("delete");
    assert_eq!(
        catalogue.resolve(&current).err(),
        Some(ResolveWorkflowError::Missing)
    );
}

#[test]
fn definition_fits_agent_rejects_unknown_tools_and_stronger_access() {
    let definition = named("Planner");
    assert!(definition_fits_agent(
        &definition,
        &[ToolId::List, ToolId::Read],
        &[("project".to_owned(), AccessMode::ReadWrite)],
        "project"
    ));
    assert!(!definition_fits_agent(
        &definition,
        &[ToolId::Read],
        &[("project".to_owned(), AccessMode::ReadWrite)],
        "project"
    ));
    assert!(!definition_fits_agent(
        &definition,
        &[ToolId::List],
        &[("project".to_owned(), AccessMode::ReadOnly)],
        "project"
    ));
    assert!(!definition_fits_agent(
        &definition,
        &[ToolId::List],
        &[("docs".to_owned(), AccessMode::ReadWrite)],
        "project"
    ));
    assert!(definition_fits_agent(
        &definition,
        &[ToolId::List, ToolId::Read],
        &[("source".to_owned(), AccessMode::ReadWrite)],
        "source"
    ));
}

#[test]
fn command_steps_do_not_require_agent_authority() {
    let role = RoleDefinition::new(
        RoleKey::parse("agent").expect("role"),
        "Coding agent".to_owned(),
        String::new(),
        String::new(),
    )
    .expect("role");
    let authority = AgentAuthority::new(vec![ToolId::List], Vec::new()).expect("authority");
    let definition = WorkflowDefinition::from_parts(
        "Mixed".to_owned(),
        test_environment_id(),
        vec![role],
        vec![
            StepDefinition {
                key: StepKey::parse("status").expect("step"),
                name: "Status".to_owned(),
                inputs: vec![initial_candidate_input()],
                action: StepAction::SystemCommand(SystemCommandStep {
                    environment: StepEnvironment::WorkflowDefault,
                    command: SystemCommandId::RepositoryStatus,
                    required_outputs: Vec::new(),
                }),
                on_success: SuccessTransition::Next(StepKey::parse("work").expect("next")),
            },
            StepDefinition {
                key: StepKey::parse("work").expect("step"),
                name: "Work".to_owned(),
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
            },
        ],
    )
    .expect("definition");
    assert!(definition_fits_agent(
        &definition,
        &[ToolId::List],
        &[("project".to_owned(), AccessMode::ReadWrite)],
        "project"
    ));
}

#[test]
fn earlier_catalogue_file_versions_are_rejected() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let json = serde_json::json!({
        "file-version": 0,
        "applied-seeds": [],
        "retired-workflow-ids": [],
        "workflows": []
    });
    write_catalogue(&path, &json.to_string());
    assert_eq!(
        WorkflowCatalogue::open_with_seeds(path, &[]).err(),
        Some(CatalogueError::Corrupt)
    );
}
