use super::{
    ONE_AGENT_V1, SEQUENTIAL_TEAM_V1, SeedKey, WorkflowSeed, one_agent_definition,
    review_with_fixes_definition, sequential_team_definition,
};
use crate::agents::ToolId;
use crate::workflows::catalogue::WorkflowCatalogue;
use crate::workflows::commands::{CommandSourceEffect, SystemCommandId};
use crate::workflows::definition::{
    ArtefactKind, ArtefactSource, OutputKind, StepAction, StepEnvironment, WorkflowDefinition,
    test_environment_id,
};

fn named(name: &str) -> WorkflowDefinition {
    let definition = one_agent_definition(test_environment_id());
    WorkflowDefinition::from_parts(
        name.to_owned(),
        test_environment_id(),
        definition.roles().to_vec(),
        definition.first_step().clone(),
        definition.steps().to_vec(),
    )
    .expect("named")
}

fn names(catalogue: &WorkflowCatalogue) -> Vec<String> {
    let mut names: Vec<_> = catalogue
        .list()
        .into_iter()
        .map(|record| record.definition.name().to_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn first_open_seeds_ordinary_workflows_once() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let first = WorkflowCatalogue::open(path.clone(), test_environment_id()).expect("open");
    assert_eq!(
        names(&first),
        vec![
            "One agent".to_owned(),
            "Read-only review".to_owned(),
            "Review with fixes".to_owned(),
            "Sequential team".to_owned(),
        ]
    );
    assert_eq!(first.applied_seed_count(), 4);
    let ids: Vec<_> = first.list().into_iter().map(|record| record.id).collect();
    let second = WorkflowCatalogue::open(path, test_environment_id()).expect("reopen");
    assert_eq!(second.list().len(), 4);
    assert_eq!(second.applied_seed_count(), 4);
    let reopened: Vec<_> = second.list().into_iter().map(|record| record.id).collect();
    assert_eq!(reopened, ids);
}

#[test]
fn restart_preserves_an_edited_seeded_workflow() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let catalogue = WorkflowCatalogue::open(path.clone(), test_environment_id()).expect("open");
    let seeded = catalogue
        .list()
        .into_iter()
        .find(|record| record.definition.name() == "Sequential team")
        .expect("seed");
    catalogue
        .update(&seeded.id, seeded.revision, named("Edited team"))
        .expect("edit");
    let reopened = WorkflowCatalogue::open(path, test_environment_id()).expect("reopen");
    let loaded = reopened.get(&seeded.id).expect("loaded");
    assert_eq!(loaded.definition.name(), "Edited team");
    assert_eq!(reopened.applied_seed_count(), 4);
}

#[test]
fn restart_does_not_restore_a_deleted_seeded_workflow() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let catalogue = WorkflowCatalogue::open(path.clone(), test_environment_id()).expect("open");
    let seeded = catalogue
        .list()
        .into_iter()
        .find(|record| record.definition.name() == "Sequential team")
        .expect("seed");
    catalogue
        .delete(&seeded.id, seeded.revision)
        .expect("delete");
    let remaining = vec![
        "One agent".to_owned(),
        "Read-only review".to_owned(),
        "Review with fixes".to_owned(),
    ];
    assert_eq!(names(&catalogue), remaining);
    assert!(catalogue.retired_ids().contains(&seeded.id));
    assert_eq!(catalogue.applied_seed_count(), 4);
    let reopened = WorkflowCatalogue::open(path, test_environment_id()).expect("reopen");
    assert_eq!(names(&reopened), remaining);
    assert!(reopened.retired_ids().contains(&seeded.id));
    assert_eq!(reopened.applied_seed_count(), 4);
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
    let reopened = WorkflowCatalogue::open(path, test_environment_id()).expect("production reopen");
    assert_eq!(
        names(&reopened),
        vec![
            "Custom".to_owned(),
            "Read-only review".to_owned(),
            "Review with fixes".to_owned(),
            "Sequential team".to_owned(),
        ]
    );
}

#[test]
fn a_name_collision_creates_a_suffixed_ordinary_record() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("workflows.json");
    let catalogue = WorkflowCatalogue::open_with_seeds(path.clone(), &[]).expect("open");
    let user = catalogue
        .create(named("Sequential team"))
        .expect("user record");
    let reopened = WorkflowCatalogue::open(path, test_environment_id()).expect("seed later");
    let records = reopened.list();
    assert!(
        records
            .iter()
            .any(|record| record.id == user.id && record.definition.name() == "Sequential team")
    );
    assert!(
        records
            .iter()
            .any(|record| record.definition.name() == "Sequential team 2")
    );
    assert!(
        records
            .iter()
            .any(|record| record.definition.name() == "One agent")
    );
}

#[test]
fn sequential_team_uses_the_supplied_environment_without_seed_provenance() {
    let environment = test_environment_id();
    let definition = sequential_team_definition(environment);
    assert_eq!(definition.default_environment(), environment);
    assert!(
        definition
            .steps()
            .iter()
            .all(|step| step.environment() == Some(StepEnvironment::WorkflowDefault))
    );
    let bytes = serde_json::to_vec(&definition.to_file()).expect("json");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(!text.contains(SEQUENTIAL_TEAM_V1));
    assert!(!text.contains(ONE_AGENT_V1));
    assert!(!text.contains("built_in"));
    assert!(!text.contains("built-in"));
}

#[test]
fn sequential_team_authority_and_handoff_match_the_validator() {
    let definition = sequential_team_definition(test_environment_id());
    assert_eq!(definition.roles().len(), 3);
    assert_eq!(definition.steps().len(), 4);
    assert_eq!(definition.first_step().as_str(), "planner");

    let planner = definition.step(definition.first_step()).expect("planner");
    let StepAction::Agent(action) = &planner.action else {
        panic!("planner");
    };
    assert_eq!(
        action.authority.tools,
        vec![ToolId::List, ToolId::Read, ToolId::Run]
    );
    assert_eq!(
        action.candidate_authority,
        crate::workflows::definition::CandidateAuthority::ReadOnly
    );
    assert!(action.authority.directories.is_empty());
    assert_eq!(
        planner.inputs[0].source,
        ArtefactSource::RunInitialCandidate
    );
    assert!(
        action
            .required_outputs
            .iter()
            .any(|output| output.kind == OutputKind::Plan)
    );

    let implementer = definition
        .step(&crate::workflows::definition::StepKey::parse("implementer").expect("key"))
        .expect("implementer");
    let StepAction::Agent(action) = &implementer.action else {
        panic!("implementer");
    };
    assert_eq!(action.authority.tools, ToolId::ALL.to_vec());
    assert_eq!(
        action.candidate_authority,
        crate::workflows::definition::CandidateAuthority::Edit
    );
    assert!(action.authority.directories.is_empty());
    assert!(implementer.inputs.iter().any(|input| {
        input.kind == ArtefactKind::Plan
            && matches!(
                input.source,
                ArtefactSource::StepOutput { ref step, .. } if step.as_str() == "planner"
            )
    }));

    let reviewer = definition
        .step(&crate::workflows::definition::StepKey::parse("reviewer").expect("key"))
        .expect("reviewer");
    let StepAction::Agent(action) = &reviewer.action else {
        panic!("reviewer");
    };
    assert_eq!(
        action.candidate_authority,
        crate::workflows::definition::CandidateAuthority::ReadOnly
    );
    assert!(action.authority.directories.is_empty());
    assert!(
        action
            .required_outputs
            .iter()
            .any(|output| output.kind == OutputKind::ReviewReport)
    );

    let commit = definition
        .step(&crate::workflows::definition::StepKey::parse("commit").expect("key"))
        .expect("commit");
    let StepAction::SystemCommand(action) = &commit.action else {
        panic!("commit");
    };
    assert_eq!(action.command, SystemCommandId::CommitCandidate);
    assert_eq!(
        commit.command_source_effect(),
        Some(CommandSourceEffect::Commit)
    );
    assert_eq!(
        action.required_outputs[0].key.as_str(),
        "committed-candidate"
    );
}

#[test]
fn review_with_fixes_uses_exact_independent_review_edges() {
    let definition = review_with_fixes_definition(test_environment_id());
    let independent = definition
        .step(&crate::workflows::definition::StepKey::parse("independent-reviewer").expect("key"))
        .expect("independent reviewer");
    assert!(independent.inputs.iter().any(|input| {
        input.kind == ArtefactKind::CandidateRevision
            && matches!(
                &input.source,
                ArtefactSource::StepOutput { step, output }
                    if step.as_str() == "fixing-reviewer" && output.as_str() == "candidate"
            )
    }));
    assert!(independent.inputs.iter().any(|input| {
        input.kind == ArtefactKind::ReviewReport
            && matches!(
                &input.source,
                ArtefactSource::StepOutput { step, output }
                    if step.as_str() == "fixing-reviewer" && output.as_str() == "review"
            )
    }));
    let commit = definition
        .step(&crate::workflows::definition::StepKey::parse("commit").expect("key"))
        .expect("commit");
    assert!(commit.inputs.iter().any(|input| {
        input.kind == ArtefactKind::CandidateRevision
            && matches!(
                &input.source,
                ArtefactSource::StepOutput { step, output }
                    if step.as_str() == "fixing-reviewer" && output.as_str() == "candidate"
            )
    }));
    assert!(commit.inputs.iter().any(|input| {
        input.kind == ArtefactKind::ReviewReport
            && matches!(
                &input.source,
                ArtefactSource::StepOutput { step, output }
                    if step.as_str() == "independent-reviewer" && output.as_str() == "review"
            )
    }));
}

#[test]
fn scheduler_inputs_contain_no_seed_provenance() {
    let definition = one_agent_definition(test_environment_id());
    let bytes = serde_json::to_vec(&definition.to_file()).expect("json");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(!text.contains(ONE_AGENT_V1));
    assert!(!text.contains("built_in"));
    assert!(!text.contains("built-in"));
    assert!(matches!(definition.steps()[0].action, StepAction::Agent(_)));
}
