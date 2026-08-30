use super::{InputContextError, format_agent_context, verify_inputs};
use crate::workflows::artefacts::{
    ArtefactProducer, ArtefactProvenance, ArtefactRecord, ArtefactReference, ArtefactSummary,
    ProductionDisposition, WorkflowArtefactRepository, artefact_hash_for, payload,
};
use crate::workflows::definition::{
    ArtefactKind, OutputKey, PinnedWorkflowDefinition, StepKey, test_environment_id,
};
use crate::workflows::id::{ArtefactId, AttemptId, RunId};
use crate::workflows::run::{AttemptArtefactInput, WorkflowRun};
use crate::workflows::seeds::sequential_team_definition;

fn store() -> WorkflowArtefactRepository {
    WorkflowArtefactRepository::in_memory()
}

fn run() -> WorkflowRun {
    let definition = sequential_team_definition(test_environment_id());
    let environments = crate::workflows::test_environment_set(&definition);
    WorkflowRun::create(
        RunId::generate().expect("run"),
        1,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    )
}

fn publish_candidate(run: &mut WorkflowRun, store: &WorkflowArtefactRepository) -> ArtefactRecord {
    let dir = tempfile::tempdir().expect("git");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .expect("git")
            .success()
    );
    std::fs::write(dir.path().join("secret-bytes.txt"), b"CANDIDATE-BYTES").expect("file");
    let captured = crate::workflows::artefacts::CandidateCapture::capture_host(dir.path(), store)
        .expect("capture");
    let bytes = captured.manifest_bytes().expect("manifest");
    let object = store.publish(&bytes).expect("object");
    let record = ArtefactRecord {
        id: ArtefactId::generate().expect("id"),
        kind: ArtefactKind::CandidateRevision,
        artefact_hash: artefact_hash_for(
            ArtefactKind::CandidateRevision,
            crate::workflows::artefacts::CANDIDATE_SCHEMA,
            &bytes,
        ),
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: 1,
        provenance: ArtefactProvenance {
            run_id: run.id,
            producer: ArtefactProducer::RunSourceCapture,
            inputs: Vec::new(),
        },
        summary: ArtefactSummary::Candidate {
            candidate: captured.candidate_hash,
            entries: captured.entries.len() as u64,
            bytes: 0,
            disposition: ProductionDisposition::RequiredOutput,
        },
    };
    run.artefacts.push(record.clone());
    record
}

fn publish_plan(
    run: &mut WorkflowRun,
    store: &WorkflowArtefactRepository,
    producer: ArtefactProducer,
    markdown: &str,
) -> ArtefactRecord {
    let (bytes, object, hash) = payload::encode_plan(markdown, None).expect("plan");
    let record = ArtefactRecord {
        id: ArtefactId::generate().expect("id"),
        kind: ArtefactKind::Plan,
        artefact_hash: hash,
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: 1,
        provenance: ArtefactProvenance {
            run_id: run.id,
            producer,
            inputs: Vec::new(),
        },
        summary: ArtefactSummary::Plan {
            markdown_bytes: markdown.len() as u64,
        },
    };
    store.publish(&bytes).expect("store");
    run.artefacts.push(record.clone());
    record
}

fn publish_review(
    run: &mut WorkflowRun,
    store: &WorkflowArtefactRepository,
    candidate: crate::workflows::artefacts::CandidateHash,
    verdict: payload::ReviewVerdict,
) -> ArtefactRecord {
    let (bytes, object, hash) =
        payload::encode_review(candidate, verdict, "Looks correct.", None).expect("review");
    let record = ArtefactRecord {
        id: ArtefactId::generate().expect("id"),
        kind: ArtefactKind::ReviewReport,
        artefact_hash: hash,
        object_hash: object,
        payload_bytes: bytes.len() as u64,
        created_at_ms: 1,
        provenance: ArtefactProvenance {
            run_id: run.id,
            producer: ArtefactProducer::StepAttempt {
                attempt_id: AttemptId::generate().expect("attempt"),
                step: StepKey::parse("reviewer").expect("step"),
                output: Some(OutputKey::parse("review").expect("output")),
                disposition: ProductionDisposition::RequiredOutput,
            },
            inputs: Vec::new(),
        },
        summary: ArtefactSummary::Review { candidate, verdict },
    };
    store.publish(&bytes).expect("store");
    run.artefacts.push(record.clone());
    record
}

fn input_of(key: &str, record: &ArtefactRecord) -> AttemptArtefactInput {
    AttemptArtefactInput {
        key: crate::workflows::definition::InputKey::parse(key).expect("key"),
        artefact: ArtefactReference {
            id: record.id,
            kind: record.kind,
            artefact_hash: record.artefact_hash,
        },
    }
}

fn planner_plan_producer() -> ArtefactProducer {
    ArtefactProducer::StepAttempt {
        attempt_id: AttemptId::generate().expect("attempt"),
        step: StepKey::parse("planner").expect("step"),
        output: Some(OutputKey::parse("plan").expect("output")),
        disposition: ProductionDisposition::RequiredOutput,
    }
}

fn implementer_candidate_producer() -> ArtefactProducer {
    ArtefactProducer::StepAttempt {
        attempt_id: AttemptId::generate().expect("attempt"),
        step: StepKey::parse("implementer").expect("step"),
        output: Some(OutputKey::parse("candidate").expect("output")),
        disposition: ProductionDisposition::RequiredOutput,
    }
}

#[test]
fn handoff_table_pins_identifiers_and_excludes_candidate_bytes() {
    let store = store();
    let mut run = run();
    let candidate = publish_candidate(&mut run, &store);
    let plan = publish_plan(&mut run, &store, planner_plan_producer(), "Do the work.");
    let implementer = run
        .pinned
        .definition
        .step(&StepKey::parse("implementer").expect("step"))
        .cloned()
        .expect("step");
    let verified = verify_inputs(
        &run,
        &implementer,
        &[input_of("candidate", &candidate), input_of("plan", &plan)],
        &store,
    )
    .expect("implementer");
    assert_eq!(verified[0].kind, ArtefactKind::CandidateRevision);
    assert_eq!(verified[0].artefact_hash, candidate.artefact_hash);
    assert_eq!(verified[0].producer_step, None);
    assert_eq!(verified[1].kind, ArtefactKind::Plan);
    assert_eq!(
        verified[1].producer_step.as_ref().map(StepKey::as_str),
        Some("planner")
    );
    assert_eq!(
        verified[1].producer_output.as_ref().map(OutputKey::as_str),
        Some("plan")
    );
    assert_eq!(verified[1].text.as_deref(), Some("Do the work."));
    let context = format_agent_context(&verified, true);
    assert!(context.contains("Do the work."));
    assert!(context.contains(&candidate.artefact_hash.as_str()));
    assert!(!context.contains("CANDIDATE-BYTES"));
    assert!(context.contains("accepted plan is task direction"));

    let mut produced = candidate.clone();
    produced.id = ArtefactId::generate().expect("id");
    produced.provenance.producer = implementer_candidate_producer();
    run.artefacts.push(produced.clone());
    let review = publish_review(
        &mut run,
        &store,
        produced.candidate_hash().expect("hash"),
        payload::ReviewVerdict::Approved,
    );
    let reviewer = run
        .pinned
        .definition
        .step(&StepKey::parse("reviewer").expect("step"))
        .cloned()
        .expect("step");
    let verified = verify_inputs(
        &run,
        &reviewer,
        &[input_of("candidate", &produced), input_of("plan", &plan)],
        &store,
    )
    .expect("reviewer");
    assert_eq!(
        verified[0].producer_step.as_ref().map(StepKey::as_str),
        Some("implementer")
    );
    let context = format_agent_context(&verified, false);
    assert!(context.contains("Assess both the accepted plan"));

    let commit = run
        .pinned
        .definition
        .step(&StepKey::parse("commit").expect("step"))
        .cloned()
        .expect("step");
    let verified = verify_inputs(
        &run,
        &commit,
        &[
            input_of("candidate", &produced),
            input_of("review", &review),
        ],
        &store,
    )
    .expect("commit");
    assert_eq!(verified[1].kind, ArtefactKind::ReviewReport);
    assert_eq!(verified[1].candidate, produced.candidate_hash());
}

#[test]
fn handoff_rejects_missing_changed_cross_run_wrong_kind_excess_and_assistant() {
    let store = store();
    let mut run = run();
    let candidate = publish_candidate(&mut run, &store);
    let planner = run
        .pinned
        .definition
        .step(&StepKey::parse("planner").expect("step"))
        .cloned()
        .expect("step");
    assert_eq!(
        verify_inputs(&run, &planner, &[], &store).err(),
        Some(InputContextError::Missing)
    );

    let mut changed = input_of("candidate", &candidate);
    changed.artefact.artefact_hash =
        crate::workflows::artefacts::ArtefactHash::parse(&format!("sha256:{}", "ab".repeat(32)))
            .expect("hash");
    assert_eq!(
        verify_inputs(&run, &planner, &[changed], &store).err(),
        Some(InputContextError::Changed)
    );

    let mut foreign = candidate.clone();
    foreign.id = ArtefactId::generate().expect("id");
    foreign.provenance.run_id = RunId::generate().expect("run");
    run.artefacts.push(foreign.clone());
    assert_eq!(
        verify_inputs(&run, &planner, &[input_of("candidate", &foreign)], &store).err(),
        Some(InputContextError::Provenance)
    );

    let plan = publish_plan(&mut run, &store, planner_plan_producer(), "plan");
    let mut wrong = input_of("candidate", &plan);
    wrong.key = crate::workflows::definition::InputKey::parse("candidate").expect("key");
    assert_eq!(
        verify_inputs(&run, &planner, &[wrong], &store).err(),
        Some(InputContextError::Kind)
    );

    let implementer = run
        .pinned
        .definition
        .step(&StepKey::parse("implementer").expect("step"))
        .cloned()
        .expect("step");
    let assistant = ArtefactRecord {
        id: ArtefactId::generate().expect("id"),
        kind: ArtefactKind::Plan,
        artefact_hash: plan.artefact_hash,
        object_hash: plan.object_hash,
        payload_bytes: 0,
        created_at_ms: 1,
        provenance: ArtefactProvenance {
            run_id: run.id,
            producer: ArtefactProducer::StepAttempt {
                attempt_id: AttemptId::generate().expect("attempt"),
                step: StepKey::parse("planner").expect("step"),
                output: Some(OutputKey::parse("assistant-reply").expect("output")),
                disposition: ProductionDisposition::RequiredOutput,
            },
            inputs: Vec::new(),
        },
        summary: ArtefactSummary::Plan { markdown_bytes: 0 },
    };
    run.artefacts.push(assistant.clone());
    assert_eq!(
        verify_inputs(
            &run,
            &implementer,
            &[
                input_of("candidate", &candidate),
                input_of("plan", &assistant)
            ],
            &store
        )
        .err(),
        Some(InputContextError::Source)
    );
}
