use super::{
    ActionKind, AttemptCleanupRecord, AttemptId, AttemptRecord, AttemptResult, AttemptSandboxKind,
    AttemptSandboxRecord, AttemptState, EscalationReason, FailureCategory, RunState,
    TransitionCause, TransitionError, WorkflowRun, next_ordinal_for,
};
use crate::agents::ToolId;
use crate::workflows::capabilities::{test_agent_capabilities, test_command_capabilities};
use crate::workflows::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, ArtefactKind, ArtefactSource, CandidateAuthority,
    InputKey, OutputKey, OutputKind, PinnedWorkflowDefinition, RequiredInput, RequiredOutput,
    RoleDefinition, RoleKey, StepAction, StepDefinition, StepEnvironment, StepKey, SystemCommandId,
    SystemCommandStep, WorkflowDefinition, candidate_revision_output, initial_candidate_input,
    test_environment_id,
};
use crate::workflows::id::RunId;

fn definition() -> WorkflowDefinition {
    two_step(false)
}

fn two_step(include_command: bool) -> WorkflowDefinition {
    let role = RoleDefinition::new(
        RoleKey::parse("agent").expect("role"),
        "Maintainer".to_owned(),
        String::new(),
        String::new(),
    )
    .expect("role");
    let authority = AgentAuthority::new(vec![ToolId::List], Vec::new()).expect("authority");
    let reply = StepDefinition {
        key: StepKey::parse("reply").expect("step"),
        name: "Reply".to_owned(),
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
        review: None,
    };
    let mut steps = vec![reply];
    if include_command {
        steps.push(StepDefinition {
            key: StepKey::parse("status").expect("step"),
            name: "Status".to_owned(),
            inputs: vec![RequiredInput {
                key: InputKey::parse("candidate").expect("input"),
                kind: ArtefactKind::CandidateRevision,
                source: ArtefactSource::StepOutput {
                    step: StepKey::parse("reply").expect("step"),
                    output: OutputKey::parse("candidate").expect("output"),
                },
            }],
            action: StepAction::SystemCommand(SystemCommandStep {
                environment: StepEnvironment::WorkflowDefault,
                command: SystemCommandId::RepositoryStatus,
                required_outputs: Vec::new(),
            }),
            review: None,
        });
    }
    WorkflowDefinition::from_parts(
        "Maintainer".to_owned(),
        test_environment_id(),
        vec![role],
        steps,
    )
    .expect("definition")
}

fn new_run() -> WorkflowRun {
    let definition = definition();
    let environments = crate::workflows::test_environment_set(&definition);
    WorkflowRun::create(
        RunId::generate().expect("run"),
        10,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    )
}

fn sandbox_record(run: &WorkflowRun) -> AttemptSandboxRecord {
    let digest = run.environments.steps[0].snapshot_digest.clone();
    AttemptSandboxRecord {
        kind: AttemptSandboxKind::IsolatedAttempt,
        snapshot_digest: digest,
    }
}

fn start(run: &mut WorkflowRun) -> AttemptId {
    let attempt = AttemptId::generate().expect("attempt");
    let caps = match run.pinned.definition.step(run.ready_step().expect("step")) {
        Some(step)
            if matches!(
                step.action,
                crate::workflows::definition::StepAction::SystemCommand(_)
            ) =>
        {
            test_command_capabilities()
        }
        _ => test_agent_capabilities(),
    };
    run.start_attempt(attempt, Vec::new(), caps, sandbox_record(run), 11)
        .expect("start");
    attempt
}

fn complete(run: &mut WorkflowRun, attempt: AttemptId, at_ms: u64) {
    run.record_cleanup(attempt, AttemptCleanupRecord::Complete)
        .expect("cleanup");
    run.complete_attempt(attempt, at_ms).expect("complete");
}

fn fail(run: &mut WorkflowRun, attempt: AttemptId, category: FailureCategory, at_ms: u64) {
    run.record_cleanup(attempt, AttemptCleanupRecord::Complete)
        .expect("cleanup");
    run.fail_attempt(attempt, category, at_ms).expect("fail");
}

#[test]
fn fixing_review_cross_run_and_cross_attempt_provenance_fails_load() {
    enum Corruption {
        CandidateRun,
        ReportRun,
        CandidateAttempt,
        ReportAttempt,
    }

    for corruption in [
        Corruption::CandidateRun,
        Corruption::ReportRun,
        Corruption::CandidateAttempt,
        Corruption::ReportAttempt,
    ] {
        let run = completed_fixing_review_run();
        let mut file = run.to_file();
        let candidate = file
            .artefacts
            .iter()
            .position(|record| {
                matches!(record.summary, super::SummaryFile::Candidate { .. })
                    && matches!(
                        record.provenance.producer,
                        super::ProducerFile::StepAttempt { .. }
                    )
            })
            .expect("candidate");
        let report = file
            .artefacts
            .iter()
            .position(|record| matches!(record.summary, super::SummaryFile::Review { .. }))
            .expect("report");
        let record = match corruption {
            Corruption::CandidateRun | Corruption::CandidateAttempt => {
                &mut file.artefacts[candidate]
            }
            Corruption::ReportRun | Corruption::ReportAttempt => &mut file.artefacts[report],
        };
        match corruption {
            Corruption::CandidateRun | Corruption::ReportRun => {
                record.provenance.run_id = "a".repeat(32);
            }
            Corruption::CandidateAttempt | Corruption::ReportAttempt => {
                let super::ProducerFile::StepAttempt { attempt_id, .. } =
                    &mut record.provenance.producer
                else {
                    panic!("step producer")
                };
                *attempt_id = "b".repeat(32);
            }
        }
        assert_eq!(
            WorkflowRun::from_file(file).err(),
            Some(super::RunRecordError::Corrupt)
        );
    }
}

fn completed_fixing_review_run() -> WorkflowRun {
    let role_key = RoleKey::parse("reviewer").expect("role");
    let step = StepDefinition {
        key: StepKey::parse("fixing-reviewer").expect("step"),
        name: "Fixing reviewer".to_owned(),
        inputs: vec![initial_candidate_input()],
        action: StepAction::Agent(AgentStep {
            environment: StepEnvironment::WorkflowDefault,
            role: role_key.clone(),
            candidate_authority: CandidateAuthority::Edit,
            authority: AgentAuthority::new(vec![ToolId::List], Vec::new()).expect("authority"),
            required_outputs: vec![
                RequiredOutput {
                    key: OutputKey::parse(ASSISTANT_REPLY).expect("reply"),
                    kind: OutputKind::AssistantReply,
                },
                candidate_revision_output(),
                RequiredOutput {
                    key: OutputKey::parse("review").expect("review"),
                    kind: OutputKind::ReviewReport,
                },
            ],
        }),
        review: None,
    };
    let definition = WorkflowDefinition::from_parts(
        "Fixing review".to_owned(),
        test_environment_id(),
        vec![
            RoleDefinition::new(
                role_key,
                "Reviewer".to_owned(),
                String::new(),
                String::new(),
            )
            .expect("role"),
        ],
        vec![step],
    )
    .expect("definition");
    let environments = crate::workflows::test_environment_set(&definition);
    let mut run = WorkflowRun::create(
        RunId::generate().expect("run"),
        10,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let initial = test_artefact_record(
        run.id,
        ArtefactKind::CandidateRevision,
        crate::workflows::artefacts::ArtefactProducer::RunSourceCapture,
        Vec::new(),
    );
    let initial_reference = crate::workflows::artefacts::ArtefactReference {
        id: initial.id,
        kind: initial.kind,
        artefact_hash: initial.artefact_hash,
    };
    run.record_initial_candidate(initial).expect("initial");
    let attempt = AttemptId::generate().expect("attempt");
    let inputs = vec![super::AttemptArtefactInput {
        key: InputKey::parse("candidate").expect("input"),
        artefact: initial_reference,
    }];
    run.start_attempt(
        attempt,
        inputs.clone(),
        test_agent_capabilities(),
        sandbox_record(&run),
        11,
    )
    .expect("start");
    let producer = |output: &str| crate::workflows::artefacts::ArtefactProducer::StepAttempt {
        attempt_id: attempt,
        step: StepKey::parse("fixing-reviewer").expect("step"),
        output: Some(OutputKey::parse(output).expect("output")),
        disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
    };
    let candidate = test_artefact_record(
        run.id,
        ArtefactKind::CandidateRevision,
        producer("candidate"),
        inputs.iter().map(|input| input.artefact.clone()).collect(),
    );
    let candidate_reference = crate::workflows::artefacts::ArtefactReference {
        id: candidate.id,
        kind: candidate.kind,
        artefact_hash: candidate.artefact_hash,
    };
    let report = test_artefact_record(
        run.id,
        ArtefactKind::ReviewReport,
        producer("review"),
        vec![candidate_reference.clone()],
    );
    let report_reference = crate::workflows::artefacts::ArtefactReference {
        id: report.id,
        kind: report.kind,
        artefact_hash: report.artefact_hash,
    };
    run.record_attempt_outputs(
        attempt,
        vec![candidate, report],
        vec![
            super::AttemptArtefactOutput {
                key: OutputKey::parse("candidate").expect("output"),
                artefact: candidate_reference.clone(),
            },
            super::AttemptArtefactOutput {
                key: OutputKey::parse("review").expect("output"),
                artefact: report_reference,
            },
        ],
        Some(candidate_reference.clone()),
        super::ObservedCandidate::Exact {
            artefact: candidate_reference,
        },
    )
    .expect("outputs");
    run.record_cleanup(attempt, AttemptCleanupRecord::Complete)
        .expect("cleanup");
    run.complete_attempt(attempt, 12).expect("complete");
    run
}

fn test_artefact_record(
    run_id: RunId,
    kind: ArtefactKind,
    producer: crate::workflows::artefacts::ArtefactProducer,
    inputs: Vec<crate::workflows::artefacts::ArtefactReference>,
) -> crate::workflows::artefacts::ArtefactRecord {
    let candidate = crate::workflows::artefacts::CandidateHash::of(b"candidate");
    crate::workflows::artefacts::ArtefactRecord {
        id: crate::workflows::ArtefactId::generate().expect("artefact"),
        kind,
        artefact_hash: crate::workflows::artefacts::ArtefactHash::of(
            b"test",
            kind.as_str().as_bytes(),
        ),
        object_hash: crate::workflows::artefacts::ObjectHash::of(kind.as_str().as_bytes()),
        payload_bytes: 1,
        created_at_ms: 11,
        provenance: crate::workflows::artefacts::ArtefactProvenance {
            run_id,
            producer,
            inputs,
        },
        summary: match kind {
            ArtefactKind::CandidateRevision => {
                crate::workflows::artefacts::ArtefactSummary::Candidate {
                    candidate,
                    entries: 0,
                    bytes: 0,
                    disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
                }
            }
            ArtefactKind::ReviewReport => crate::workflows::artefacts::ArtefactSummary::Review {
                candidate,
                verdict: crate::workflows::artefacts::ReviewVerdict::Approved,
            },
            _ => panic!("test artefact kind"),
        },
    }
}

#[test]
fn creation_stores_ready_without_a_transition() {
    let run = new_run();
    assert!(matches!(run.state, RunState::Ready { .. }));
    assert!(run.transitions.is_empty());
    assert!(run.attempts.is_empty());
}

#[test]
fn parallel_attempts_are_rejected() {
    let mut run = new_run();
    start(&mut run);
    let second = AttemptId::generate().expect("attempt");
    assert_eq!(
        run.start_attempt(
            second,
            Vec::new(),
            test_agent_capabilities(),
            sandbox_record(&run),
            12,
        ),
        Err(TransitionError::Invalid)
    );
    assert_eq!(run.attempts.len(), 1);
    assert_eq!(run.transitions.len(), 1);
}

#[test]
fn stale_completions_are_rejected() {
    let mut run = new_run();
    start(&mut run);
    let stale = AttemptId::generate().expect("attempt");
    assert_eq!(
        run.complete_attempt(stale, 12),
        Err(TransitionError::Invalid)
    );
    assert!(matches!(run.state, RunState::Active { .. }));
}

#[test]
fn transition_times_cannot_move_backwards() {
    let mut run = new_run();
    let attempt = start(&mut run);
    assert_eq!(
        run.fail_attempt(attempt, FailureCategory::Provider, 10),
        Err(TransitionError::Invalid)
    );
    assert!(matches!(run.state, RunState::Active { .. }));
}

#[test]
fn duplicate_terminal_results_are_rejected() {
    let mut run = new_run();
    let attempt = start(&mut run);
    complete(&mut run, attempt, 12);
    assert_eq!(
        run.complete_attempt(attempt, 13),
        Err(TransitionError::Invalid)
    );
    assert_eq!(
        run.fail_attempt(attempt, FailureCategory::Provider, 13),
        Err(TransitionError::Invalid)
    );
}

#[test]
fn transitions_after_a_terminal_state_are_rejected() {
    let mut run = new_run();
    let attempt = start(&mut run);
    fail(&mut run, attempt, FailureCategory::Provider, 12);
    assert_eq!(run.cancel(13), Err(TransitionError::Invalid));
    assert_eq!(
        run.start_attempt(
            AttemptId::generate().expect("attempt"),
            Vec::new(),
            test_agent_capabilities(),
            sandbox_record(&run),
            13,
        ),
        Err(TransitionError::Invalid)
    );
    assert_eq!(run.interrupt(13), Err(TransitionError::Invalid));
}

#[test]
fn completed_attempts_advance_by_vector_position() {
    let definition = two_step(true);
    let environments = crate::workflows::test_environment_set(&definition);
    let mut run = WorkflowRun::create(
        RunId::generate().expect("run"),
        10,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let first = AttemptId::generate().expect("attempt");
    run.start_attempt(
        first,
        Vec::new(),
        test_agent_capabilities(),
        sandbox_record(&run),
        11,
    )
    .expect("start");
    complete(&mut run, first, 12);
    assert!(matches!(run.state, RunState::Ready { ref step } if step.as_str() == "status"));
    let second = AttemptId::generate().expect("attempt");
    run.start_attempt(
        second,
        Vec::new(),
        test_command_capabilities(),
        sandbox_record(&run),
        13,
    )
    .expect("start");
    complete(&mut run, second, 14);
    assert_eq!(run.state, RunState::Completed);
}

#[test]
fn failed_attempts_move_the_run_to_failed() {
    let mut run = new_run();
    let attempt = start(&mut run);
    fail(&mut run, attempt, FailureCategory::Command, 12);
    assert_eq!(run.state, RunState::Failed);
    assert_eq!(
        run.transitions.last().map(|item| item.cause),
        Some(TransitionCause::AttemptFailed)
    );
}

#[test]
fn cancellation_moves_the_run_directly_to_cancelled() {
    let mut run = new_run();
    let attempt = start(&mut run);
    run.record_cleanup(attempt, AttemptCleanupRecord::Complete)
        .expect("cleanup");
    run.cancel(12).expect("cancel");
    assert_eq!(run.state, RunState::Cancelled);
    assert_eq!(run.attempts[0].state, AttemptState::Cancelled);
}

#[test]
fn attempt_ordinals_count_repeated_step_attempts() {
    let step = StepKey::parse("reply").expect("step");
    let first = AttemptRecord {
        id: AttemptId::generate().expect("attempt"),
        step: step.clone(),
        ordinal: 1,
        action_kind: ActionKind::Agent,
        started_at_ms: 1,
        finished_at_ms: Some(2),
        state: AttemptState::Failed,
        result: Some(AttemptResult::Failed {
            category: FailureCategory::Provider,
        }),
        inputs: Vec::new(),
        outputs: Vec::new(),
        capabilities: test_agent_capabilities(),
        sandbox: AttemptSandboxRecord {
            kind: AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: crate::environments::SnapshotDigest::parse(&format!(
                "sha256:{}",
                "a".repeat(64)
            ))
            .expect("digest"),
        },
        cleanup: AttemptCleanupRecord::Complete,
        commit_transaction: None,
        commit_result: None,
    };
    let second = AttemptRecord {
        id: AttemptId::generate().expect("attempt"),
        step: step.clone(),
        ordinal: 2,
        action_kind: ActionKind::Agent,
        started_at_ms: 3,
        finished_at_ms: None,
        state: AttemptState::Active,
        result: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        capabilities: test_agent_capabilities(),
        sandbox: AttemptSandboxRecord {
            kind: AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: crate::environments::SnapshotDigest::parse(&format!(
                "sha256:{}",
                "a".repeat(64)
            ))
            .expect("digest"),
        },
        cleanup: AttemptCleanupRecord::Pending,
        commit_transaction: None,
        commit_result: None,
    };
    assert_eq!(next_ordinal_for(&[], &step), 1);
    assert_eq!(next_ordinal_for(std::slice::from_ref(&first), &step), 2);
    assert_eq!(next_ordinal_for(&[first, second], &step), 3);
}

fn artefact_reference(
    record: &crate::workflows::artefacts::ArtefactRecord,
) -> crate::workflows::artefacts::ArtefactReference {
    crate::workflows::artefacts::ArtefactReference {
        id: record.id,
        kind: record.kind,
        artefact_hash: record.artefact_hash,
    }
}

fn review_loop_run() -> WorkflowRun {
    review_run(crate::workflows::seeds::review_until_approved_definition(
        test_environment_id(),
    ))
}

fn review_run(definition: WorkflowDefinition) -> WorkflowRun {
    let environments = crate::workflows::test_environment_set(&definition);
    let mut run = WorkflowRun::create(
        RunId::generate().expect("run"),
        10,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let initial = test_artefact_record(
        run.id,
        ArtefactKind::CandidateRevision,
        crate::workflows::artefacts::ArtefactProducer::RunSourceCapture,
        Vec::new(),
    );
    run.record_initial_candidate(initial).expect("initial");
    run
}

fn step_sandbox(run: &WorkflowRun, step: &StepKey) -> AttemptSandboxRecord {
    AttemptSandboxRecord {
        kind: AttemptSandboxKind::IsolatedAttempt,
        snapshot_digest: run
            .environments
            .steps
            .iter()
            .find(|binding| &binding.step == step)
            .expect("step environment")
            .snapshot_digest
            .clone(),
    }
}

fn current_candidate(run: &WorkflowRun) -> crate::workflows::artefacts::ArtefactReference {
    let super::RunSource::Captured { source } = &run.source else {
        panic!("captured source")
    };
    source.accepted.clone()
}

fn complete_implementation(
    run: &mut WorkflowRun,
    at_ms: u64,
) -> (AttemptId, crate::workflows::artefacts::ArtefactReference) {
    let step = StepKey::parse("implementer").expect("step");
    assert_eq!(run.ready_step(), Some(&step));
    let input = current_candidate(run);
    let inputs = vec![super::AttemptArtefactInput {
        key: InputKey::parse("candidate").expect("input"),
        artefact: input.clone(),
    }];
    let attempt = AttemptId::generate().expect("attempt");
    run.start_attempt(
        attempt,
        inputs.clone(),
        test_agent_capabilities(),
        step_sandbox(run, &step),
        at_ms,
    )
    .expect("start implementation");
    let candidate = test_artefact_record(
        run.id,
        ArtefactKind::CandidateRevision,
        crate::workflows::artefacts::ArtefactProducer::StepAttempt {
            attempt_id: attempt,
            step,
            output: Some(OutputKey::parse("candidate").expect("output")),
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
        vec![input],
    );
    let reference = artefact_reference(&candidate);
    run.record_attempt_outputs(
        attempt,
        vec![candidate],
        vec![super::AttemptArtefactOutput {
            key: OutputKey::parse("candidate").expect("output"),
            artefact: reference.clone(),
        }],
        Some(reference.clone()),
        super::ObservedCandidate::Exact {
            artefact: reference.clone(),
        },
    )
    .expect("implementation output");
    run.record_cleanup(attempt, AttemptCleanupRecord::Complete)
        .expect("cleanup");
    run.complete_attempt(attempt, at_ms + 1)
        .expect("complete implementation");
    (attempt, reference)
}

fn complete_review(
    run: &mut WorkflowRun,
    verdict: crate::workflows::artefacts::ReviewVerdict,
    at_ms: u64,
) -> crate::workflows::artefacts::ArtefactReference {
    let step = StepKey::parse("reviewer").expect("step");
    assert_eq!(run.ready_step(), Some(&step));
    let input = current_candidate(run);
    let inputs = vec![super::AttemptArtefactInput {
        key: InputKey::parse("candidate").expect("input"),
        artefact: input.clone(),
    }];
    let attempt = AttemptId::generate().expect("attempt");
    let mut capabilities = test_agent_capabilities();
    capabilities.directories[0].access = crate::agents::AccessMode::ReadOnly;
    run.start_attempt(
        attempt,
        inputs,
        capabilities,
        step_sandbox(run, &step),
        at_ms,
    )
    .expect("start review");
    let mut report = test_artefact_record(
        run.id,
        ArtefactKind::ReviewReport,
        crate::workflows::artefacts::ArtefactProducer::StepAttempt {
            attempt_id: attempt,
            step,
            output: Some(OutputKey::parse("review").expect("output")),
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
        vec![input.clone()],
    );
    let crate::workflows::artefacts::ArtefactSummary::Review {
        verdict: report_verdict,
        ..
    } = &mut report.summary
    else {
        panic!("review summary")
    };
    *report_verdict = verdict;
    let reference = artefact_reference(&report);
    run.record_attempt_outputs(
        attempt,
        vec![report],
        vec![super::AttemptArtefactOutput {
            key: OutputKey::parse("review").expect("output"),
            artefact: reference.clone(),
        }],
        None,
        super::ObservedCandidate::Exact { artefact: input },
    )
    .expect("review output");
    run.record_cleanup(attempt, AttemptCleanupRecord::Complete)
        .expect("cleanup");
    run.complete_attempt(attempt, at_ms + 1)
        .expect("complete review");
    reference
}

#[test]
fn review_verdicts_select_all_four_routes() {
    use crate::workflows::artefacts::ReviewVerdict;

    enum ExpectedRoute {
        Approved,
        Revision,
        Blocked,
        AttemptLimit,
    }

    for (verdict, prior_revisions, expected) in [
        (ReviewVerdict::Approved, 0, ExpectedRoute::Approved),
        (ReviewVerdict::RevisionRequired, 0, ExpectedRoute::Revision),
        (ReviewVerdict::Blocked, 0, ExpectedRoute::Blocked),
        (
            ReviewVerdict::RevisionRequired,
            2,
            ExpectedRoute::AttemptLimit,
        ),
    ] {
        let mut run = review_loop_run();
        let mut time = 11;
        complete_implementation(&mut run, time);
        time += 2;
        for _ in 0..prior_revisions {
            complete_review(&mut run, ReviewVerdict::RevisionRequired, time);
            time += 2;
            complete_implementation(&mut run, time);
            time += 2;
        }
        let report = complete_review(&mut run, verdict, time);
        match expected {
            ExpectedRoute::Approved => {
                assert!(
                    matches!(&run.state, RunState::Ready { step } if step.as_str() == "commit")
                );
                assert_eq!(
                    run.transitions.last().map(|transition| transition.cause),
                    Some(TransitionCause::ReviewApproved)
                );
            }
            ExpectedRoute::Revision => {
                assert!(
                    matches!(&run.state, RunState::Ready { step } if step.as_str() == "implementer")
                );
                assert_eq!(
                    run.transitions.last().map(|transition| transition.cause),
                    Some(TransitionCause::ReviewRevision)
                );
            }
            ExpectedRoute::Blocked => assert_eq!(
                run.state,
                RunState::Escalated {
                    step: StepKey::parse("reviewer").expect("step"),
                    report,
                    reason: EscalationReason::Blocked,
                }
            ),
            ExpectedRoute::AttemptLimit => assert_eq!(
                run.state,
                RunState::Escalated {
                    step: StepKey::parse("reviewer").expect("step"),
                    report,
                    reason: EscalationReason::AttemptLimit,
                }
            ),
        }
    }
}

#[test]
fn final_review_approval_completes_the_run() {
    let source = crate::workflows::seeds::review_until_approved_definition(test_environment_id());
    let definition = WorkflowDefinition::from_parts(
        source.name().to_owned(),
        source.default_environment(),
        source.roles().to_vec(),
        source.steps()[..2].to_vec(),
    )
    .expect("final review definition");
    let mut run = review_run(definition);

    complete_implementation(&mut run, 11);
    complete_review(
        &mut run,
        crate::workflows::artefacts::ReviewVerdict::Approved,
        13,
    );

    assert_eq!(run.state, RunState::Completed);
    assert_eq!(
        run.transitions.last().map(|transition| transition.cause),
        Some(TransitionCause::ReviewApproved)
    );
}

#[test]
fn repeated_steps_pin_the_current_candidate_and_increment_ordinals() {
    let mut run = review_loop_run();
    let (first_attempt, first_candidate) = complete_implementation(&mut run, 11);
    complete_review(
        &mut run,
        crate::workflows::artefacts::ReviewVerdict::RevisionRequired,
        13,
    );
    let input_candidate = current_candidate(&run);
    let (second_attempt, second_candidate) = complete_implementation(&mut run, 15);
    let repeated = run
        .attempts
        .iter()
        .find(|attempt| attempt.id == second_attempt)
        .expect("repeated attempt");

    assert_eq!(repeated.ordinal, 2);
    assert_eq!(repeated.inputs[0].artefact, input_candidate);
    assert_eq!(input_candidate, first_candidate);
    assert_ne!(second_candidate.id, first_candidate.id);
    assert_ne!(second_attempt, first_attempt);
}

#[test]
fn durable_review_routes_reject_altered_transition_and_escalation_facts() {
    let mut approved = review_loop_run();
    complete_implementation(&mut approved, 11);
    complete_review(
        &mut approved,
        crate::workflows::artefacts::ReviewVerdict::Approved,
        13,
    );
    let mut altered_transition = approved.to_file();
    altered_transition
        .transitions
        .last_mut()
        .expect("transition")
        .cause = "review-revision".to_owned();
    assert_eq!(
        WorkflowRun::from_file(altered_transition).err(),
        Some(super::RunRecordError::Corrupt)
    );

    let mut blocked = review_loop_run();
    complete_implementation(&mut blocked, 11);
    complete_review(
        &mut blocked,
        crate::workflows::artefacts::ReviewVerdict::Blocked,
        13,
    );
    let mut altered_escalation = blocked.to_file();
    let super::RunStateFile::Escalated { reason, .. } = &mut altered_escalation.state else {
        panic!("escalated state")
    };
    *reason = "attempt-limit".to_owned();
    assert_eq!(
        WorkflowRun::from_file(altered_escalation).err(),
        Some(super::RunRecordError::Corrupt)
    );
}

fn reference_for(
    kind: ArtefactKind,
    marker: &[u8],
) -> crate::workflows::artefacts::ArtefactReference {
    crate::workflows::artefacts::ArtefactReference {
        id: crate::workflows::ArtefactId::generate().expect("artefact"),
        kind,
        artefact_hash: crate::workflows::artefacts::ArtefactHash::of(
            marker,
            kind.as_str().as_bytes(),
        ),
    }
}

#[test]
fn durable_commit_transactions_preserve_every_review_reference() {
    let candidate = reference_for(ArtefactKind::CandidateRevision, b"candidate");
    let first_review = reference_for(ArtefactKind::ReviewReport, b"first");
    let second_review = reference_for(ArtefactKind::ReviewReport, b"second");
    let attempt = AttemptRecord {
        id: AttemptId::generate().expect("attempt"),
        step: StepKey::parse("commit").expect("step"),
        ordinal: 1,
        action_kind: ActionKind::SystemCommand,
        started_at_ms: 1,
        finished_at_ms: None,
        state: AttemptState::Active,
        result: None,
        inputs: vec![
            super::AttemptArtefactInput {
                key: InputKey::parse("candidate").expect("input"),
                artefact: candidate.clone(),
            },
            super::AttemptArtefactInput {
                key: InputKey::parse("correctness").expect("input"),
                artefact: first_review.clone(),
            },
            super::AttemptArtefactInput {
                key: InputKey::parse("security").expect("input"),
                artefact: second_review.clone(),
            },
        ],
        outputs: Vec::new(),
        capabilities: test_command_capabilities(),
        sandbox: AttemptSandboxRecord {
            kind: AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: crate::environments::SnapshotDigest::parse(&format!(
                "sha256:{}",
                "a".repeat(64)
            ))
            .expect("digest"),
        },
        cleanup: AttemptCleanupRecord::Pending,
        commit_transaction: None,
        commit_result: None,
    };
    let transaction = crate::workflows::commit::CommitTransaction {
        state: crate::workflows::commit::CommitTransactionState::Prepared,
        candidate,
        reviews: vec![first_review.clone(), second_review.clone()],
        approval: None,
        expected_reference: "refs/heads/main".to_owned(),
        old_object: None,
        target_tree: None,
        expected_commit: None,
        timestamp: "1700000000 +0000".to_owned(),
    };
    assert!(super::valid_commit_transaction(&attempt, &transaction));

    let mut missing = transaction.clone();
    missing.reviews.pop();
    assert!(!super::valid_commit_transaction(&attempt, &missing));

    let mut duplicate = transaction;
    duplicate.reviews = vec![first_review.clone(), first_review];
    assert!(!super::valid_commit_transaction(&attempt, &duplicate));
}
