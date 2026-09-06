use super::{
    InputContextError, ProjectInstructions, build_attempt_packet, format_agent_context,
    validate_launch_brief, verify_inputs,
};
use crate::tests::test_environment_id;
use crate::workflows::artefacts::{
    ArtefactProducer, ArtefactProvenance, ArtefactRecord, ArtefactReference, ArtefactSummary,
    ProductionDisposition, WorkflowArtefactRepository, artefact_hash_for, payload,
};
use crate::workflows::definition::{ArtefactKind, OutputKey, PinnedWorkflowDefinition, StepKey};
use crate::workflows::id::{ArtefactId, AttemptId, RunId};
use crate::workflows::run::{
    AttemptArtefactInput, ObservedCandidate, RunSource, RunSourceState, WorkflowRun,
};
use crate::workflows::seeds::sequential_team_definition;

fn store() -> WorkflowArtefactRepository {
    WorkflowArtefactRepository::in_memory()
}

fn run() -> WorkflowRun {
    let definition = sequential_team_definition(test_environment_id());
    let environments = crate::tests::test_environment_set(&definition);
    WorkflowRun::configured(
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
fn current_candidate_inputs_match_the_exact_accepted_reference() {
    let store = store();
    let definition =
        crate::workflows::seeds::review_until_approved_definition(test_environment_id());
    let environments = crate::tests::test_environment_set(&definition);
    let mut run = WorkflowRun::configured(
        RunId::generate().expect("run"),
        1,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let initial = publish_candidate(&mut run, &store);
    let mut current = initial.clone();
    current.id = ArtefactId::generate().expect("current");
    current.provenance.producer = implementer_candidate_producer();
    run.artefacts.push(current.clone());
    let initial_reference = input_of("candidate", &initial).artefact;
    let current_reference = input_of("candidate", &current).artefact;
    run.source = RunSource::Captured {
        source: RunSourceState {
            initial: initial_reference.clone(),
            accepted: current_reference.clone(),
            observed: ObservedCandidate::Exact {
                artefact: current_reference,
            },
        },
    };
    let reviewer = run
        .pinned
        .definition
        .step(&StepKey::parse("reviewer").expect("step"))
        .cloned()
        .expect("reviewer");

    assert!(verify_inputs(&run, &reviewer, &[input_of("candidate", &current)], &store,).is_ok());
    assert_eq!(
        verify_inputs(
            &run,
            &reviewer,
            &[AttemptArtefactInput {
                key: crate::workflows::definition::InputKey::parse("candidate").expect("input"),
                artefact: initial_reference,
            }],
            &store,
        )
        .err(),
        Some(InputContextError::Source)
    );
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

#[test]
fn root_instruction_source_distinguishes_absence_from_a_failed_read() {
    assert_eq!(
        super::classify_instruction_exit(Some(3), true).expect("absent"),
        Some(super::ProjectInstructions::Absent)
    );
    assert_eq!(
        super::classify_instruction_exit(Some(1), true).err(),
        Some(super::InstructionError::Read)
    );
    assert_eq!(
        super::classify_instruction_exit(Some(0), false).expect("present"),
        None
    );
}

#[test]
fn root_instruction_paths_stay_inside_the_target_candidate() {
    let candidate = tempfile::tempdir().expect("candidate");
    let outside = tempfile::tempdir().expect("outside");
    let secret = outside.path().join("instructions");
    std::fs::write(&secret, "outside instructions").expect("write");
    let path = candidate.path().join("AGENTS.md");
    let read = || {
        std::process::Command::new("sh")
            .args(["-c", super::INSTRUCTION_READ_COMMAND])
            .current_dir(candidate.path())
            .output()
            .expect("read instructions")
    };
    assert_eq!(read().status.code(), Some(3));
    std::os::unix::fs::symlink(&secret, &path).expect("link");
    let escaped = read();
    assert_eq!(escaped.status.code(), Some(4));
    assert!(escaped.stdout.is_empty());
    std::fs::remove_file(&secret).expect("remove target");
    assert_eq!(read().status.code(), Some(4));
    std::fs::remove_file(&path).expect("remove link");
    std::fs::create_dir(&path).expect("directory");
    assert_eq!(read().status.code(), Some(1));
    std::fs::remove_dir(&path).expect("remove directory");
    std::fs::write(&path, "candidate instructions").expect("write candidate");
    assert_eq!(read().stdout, b"candidate instructions");
    std::fs::write(
        &path,
        "x".repeat(super::MAXIMUM_PROJECT_INSTRUCTION_BYTES + 100),
    )
    .expect("oversized instructions");
    assert_eq!(
        read().stdout.len(),
        super::MAXIMUM_PROJECT_INSTRUCTION_BYTES + 1
    );
}

#[test]
fn attempt_packets_reject_substituted_inputs_and_count_project_instructions() {
    let store = store();
    let mut run = run();
    run.launch_brief = "Inspect the repository and report risks.".to_owned();
    let candidate = publish_candidate(&mut run, &store);
    let step = run
        .pinned
        .definition
        .step(&StepKey::parse("planner").expect("step"))
        .expect("planner");
    let input = input_of("candidate", &candidate);
    let packet = build_attempt_packet(
        &run,
        step,
        std::slice::from_ref(&input),
        &store,
        ProjectInstructions::Present("Use the project test command.".to_owned()),
    )
    .expect("packet");
    let text = packet.text();
    assert!(text.contains(&run.launch_brief));
    assert!(text.contains("Use the project test command."));
    assert!(!text.contains("CANDIDATE-BYTES"));
    let instruction_bytes = super::MAXIMUM_ATTEMPT_PACKET_BYTES - packet.byte_len();
    assert_eq!(
        build_attempt_packet(
            &run,
            step,
            std::slice::from_ref(&input),
            &store,
            ProjectInstructions::Present("x".repeat(instruction_bytes + 100))
        )
        .err(),
        Some(InputContextError::Packet)
    );
    let mut changed = input;
    changed.artefact.id = ArtefactId::generate().expect("foreign artefact");
    assert!(
        build_attempt_packet(&run, step, &[changed], &store, ProjectInstructions::Absent).is_err()
    );
}

#[test]
fn launch_briefs_reject_controls_and_oversized_input() {
    assert_eq!(
        validate_launch_brief("  Inspect the project.  ").expect("brief"),
        "Inspect the project."
    );
    assert_eq!(
        validate_launch_brief("bad\u{0000}brief").err(),
        Some(InputContextError::Brief)
    );
    assert_eq!(
        validate_launch_brief(&"x".repeat(super::MAXIMUM_LAUNCH_BRIEF_BYTES + 1)).err(),
        Some(InputContextError::Brief)
    );
}

#[test]
fn root_instruction_text_rejects_invalid_oversized_and_secret_content() {
    assert!(super::validate_instruction_text("Use Australian English.\n", None).is_ok());
    assert_eq!(
        super::validate_instruction_text("bad\u{0000}text", None).err(),
        Some(super::InstructionError::Invalid)
    );
    assert_eq!(
        super::validate_instruction_text(
            &"x".repeat(super::MAXIMUM_PROJECT_INSTRUCTION_BYTES + 1),
            None,
        )
        .err(),
        Some(super::InstructionError::Bound)
    );
    assert_eq!(
        super::validate_instruction_text("use sk-secret", Some("sk-secret")).err(),
        Some(super::InstructionError::Credential)
    );
}
