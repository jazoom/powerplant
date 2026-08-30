use serde::{Deserialize, Serialize};

use crate::agents::{AccessMode, AgentId, ToolId};
use crate::environments::snapshot::{OciManifestDigest, RecordedIntegrity, SnapshotArtifactKey};
use crate::environments::{
    EnvironmentId, EnvironmentRecipeVersion, PreparationId, PreparedSnapshot, SnapshotDigest,
};

use super::artefacts::{ArtefactRecord, ArtefactReference};
use super::capabilities::{
    AttemptCapabilities, CapabilityDirectory, DirectoryRole, NetworkCapability,
    PrimarySourceLocation, SecretPresence,
};
use super::commit::{CommitResult, CommitTransaction, CommitTransactionState};
use super::definition::{
    DefinitionFile, DefinitionVersion, InputKey, OutputKey, PinnedWorkflowDefinition, StepAction,
    StepDefinition, StepKey, SuccessTransition, WorkflowDefinition,
};
use super::gates::{GateRevision, HumanGateRecord, HumanGateState};
use super::id::{AttemptId, GateId, RunId, WorkflowId};
use super::resolve::{ResolvedEnvironment, ResolvedEnvironmentSet, ResolvedStepEnvironment};

pub(crate) const RUN_RECORD_VERSION: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkflowRun {
    pub(crate) id: RunId,
    pub(crate) created_at_ms: u64,
    pub(crate) agent_id: AgentId,
    pub(crate) pinned: PinnedWorkflowDefinition,
    pub(crate) environments: ResolvedEnvironmentSet,
    pub(crate) state: RunState,
    pub(crate) source: RunSource,
    pub(crate) artefacts: Vec<ArtefactRecord>,
    pub(crate) attempts: Vec<AttemptRecord>,
    pub(crate) gates: Vec<HumanGateRecord>,
    pub(crate) transitions: Vec<TransitionRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RunState {
    InitialisingSource,
    Ready {
        step: StepKey,
    },
    Active {
        step: StepKey,
        attempt: AttemptId,
    },
    AwaitingHuman {
        step: StepKey,
        gate: GateId,
    },
    RevisionRequested {
        step: StepKey,
        decision: ArtefactReference,
    },
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RunSource {
    Pending,
    Captured { source: RunSourceState },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunSourceState {
    pub(crate) initial: ArtefactReference,
    pub(crate) accepted: ArtefactReference,
    pub(crate) observed: ObservedCandidate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ObservedCandidate {
    Exact { artefact: ArtefactReference },
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttemptArtefactInput {
    pub(crate) key: InputKey,
    pub(crate) artefact: ArtefactReference,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttemptArtefactOutput {
    pub(crate) key: OutputKey,
    pub(crate) artefact: ArtefactReference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttemptState {
    Active,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionCause {
    InitialSourceCaptureStarted,
    InitialSourceCaptured,
    InitialSourceCaptureFailed,
    SourceDriftDetected,
    AttemptStarted,
    AttemptCompleted,
    AttemptFailed,
    CancellationRequested,
    ProcessRestarted,
    GateOpened,
    GateApproved,
    GateRevisionRequested,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransitionRecord {
    pub(crate) sequence: u64,
    pub(crate) occurred_at_ms: u64,
    pub(crate) cause: TransitionCause,
    pub(crate) from: RunState,
    pub(crate) to: RunState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttemptRecord {
    pub(crate) id: AttemptId,
    pub(crate) step: StepKey,
    pub(crate) ordinal: u32,
    pub(crate) action_kind: ActionKind,
    pub(crate) started_at_ms: u64,
    pub(crate) finished_at_ms: Option<u64>,
    pub(crate) state: AttemptState,
    pub(crate) result: Option<AttemptResult>,
    pub(crate) inputs: Vec<AttemptArtefactInput>,
    pub(crate) outputs: Vec<AttemptArtefactOutput>,
    pub(crate) capabilities: AttemptCapabilities,
    pub(crate) sandbox: AttemptSandboxRecord,
    pub(crate) cleanup: AttemptCleanupRecord,
    pub(crate) commit_transaction: Option<CommitTransaction>,
    pub(crate) commit_result: Option<CommitResult>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionKind {
    Agent,
    SystemCommand,
    HumanGate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AttemptResult {
    Completed { outputs: Vec<String> },
    Failed { category: FailureCategory },
    Cancelled,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureCategory {
    Provider,
    Tool,
    Authority,
    Command,
    Operational,
    Definition,
    Cleanup,
    Assurance,
    Commit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttemptSandboxRecord {
    pub(crate) kind: AttemptSandboxKind,
    pub(crate) snapshot_digest: SnapshotDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttemptSandboxKind {
    IsolatedAttempt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum AttemptCleanupRecord {
    Pending,
    Complete,
    Orphaned {
        sandbox: bool,
        workspace: bool,
        journal: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionError {
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunRecordError {
    Corrupt,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) struct RunFile {
    record_version: u32,
    id: String,
    created_at_ms: u64,
    agent_id: String,
    workflow_id: Option<String>,
    version: String,
    definition: DefinitionFile,
    environments: ResolvedEnvironmentSetFile,
    state: RunStateFile,
    source: RunSourceFile,
    artefacts: Vec<ArtefactFile>,
    attempts: Vec<AttemptFile>,
    gates: Vec<HumanGateFile>,
    transitions: Vec<TransitionFile>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum RunStateFile {
    InitialisingSource,
    Ready {
        step: String,
    },
    Active {
        step: String,
        attempt: String,
    },
    AwaitingHuman {
        step: String,
        gate: String,
    },
    RevisionRequested {
        step: String,
        decision: ArtefactRefFile,
    },
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct HumanGateFile {
    id: String,
    step: String,
    sequence: u32,
    revision: u64,
    opened_at_ms: u64,
    closed_at_ms: Option<u64>,
    candidate: ArtefactRefFile,
    diff_base: ArtefactRefFile,
    state: String,
    decision: Option<ArtefactRefFile>,
    output: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct AttemptFile {
    id: String,
    step: String,
    ordinal: u32,
    action_kind: String,
    started_at_ms: u64,
    finished_at_ms: Option<u64>,
    state: String,
    result: Option<AttemptResultFile>,
    inputs: Vec<AttemptInputFile>,
    outputs: Vec<AttemptOutputFile>,
    capabilities: AttemptCapabilitiesFile,
    sandbox: AttemptSandboxFile,
    cleanup: AttemptCleanupFile,
    commit_transaction: Option<CommitTransactionFile>,
    commit_result: Option<CommitResultFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct CommitTransactionFile {
    state: String,
    candidate: ArtefactRefFile,
    review: ArtefactRefFile,
    approval: Option<ArtefactRefFile>,
    expected_reference: String,
    old_object: Option<String>,
    target_tree: Option<String>,
    expected_commit: Option<String>,
    timestamp: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct CommitResultFile {
    commit: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct AttemptCapabilitiesFile {
    tools: Vec<String>,
    directories: Vec<CapabilityDirectoryFile>,
    git_admin: String,
    source_location: String,
    network: String,
    secret: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct CapabilityDirectoryFile {
    alias: String,
    guest_path: String,
    access: String,
    role: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct AttemptSandboxFile {
    kind: String,
    snapshot_digest: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum AttemptCleanupFile {
    Pending,
    Complete,
    Orphaned {
        sandbox: bool,
        workspace: bool,
        journal: bool,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct AttemptInputFile {
    key: String,
    artefact: ArtefactRefFile,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct AttemptOutputFile {
    key: String,
    artefact: ArtefactRefFile,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ArtefactRefFile {
    id: String,
    kind: String,
    artefact_hash: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
#[allow(clippy::large_enum_variant)]
enum RunSourceFile {
    Pending,
    Captured { source: RunSourceStateFile },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct RunSourceStateFile {
    initial: ArtefactRefFile,
    accepted: ArtefactRefFile,
    observed: ObservedCandidateFile,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum ObservedCandidateFile {
    Exact { artefact: ArtefactRefFile },
    Unknown,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ArtefactFile {
    id: String,
    kind: String,
    artefact_hash: String,
    object_hash: String,
    payload_bytes: u64,
    created_at_ms: u64,
    provenance: ProvenanceFile,
    summary: SummaryFile,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ProvenanceFile {
    run_id: String,
    producer: ProducerFile,
    inputs: Vec<ArtefactRefFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "producer", rename_all = "kebab-case")]
enum ProducerFile {
    RunSourceCapture,
    StepAttempt {
        attempt_id: String,
        step: String,
        output: Option<String>,
        disposition: String,
    },
    HumanGate {
        gate_id: String,
        step: String,
        output: String,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum SummaryFile {
    Plan {
        markdown_bytes: u64,
    },
    Review {
        candidate: String,
        verdict: String,
    },
    Test {
        candidate: String,
        outcome: String,
    },
    Candidate {
        candidate: String,
        entries: u64,
        bytes: u64,
        disposition: String,
    },
    HumanDecision {
        candidate: String,
        diff_base: String,
        decision: String,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum AttemptResultFile {
    Completed { outputs: Vec<String> },
    Failed { category: String },
    Cancelled,
    Interrupted,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct TransitionFile {
    sequence: u64,
    occurred_at_ms: u64,
    cause: String,
    from: RunStateFile,
    to: RunStateFile,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ResolvedEnvironmentSetFile {
    environments: Vec<ResolvedEnvironmentFile>,
    steps: Vec<ResolvedStepEnvironmentFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ResolvedEnvironmentFile {
    environment_id: String,
    name: String,
    preparation_id: String,
    recipe_version: String,
    snapshot: PinnedSnapshotFile,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ResolvedStepEnvironmentFile {
    step: String,
    environment_id: String,
    preparation_id: String,
    snapshot_digest: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct PinnedSnapshotFile {
    artifact_key: String,
    snapshot_digest: String,
    image_reference: String,
    image_manifest_digest: String,
    upper_integrity: PinnedIntegrityFile,
    upper_size_bytes: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct PinnedIntegrityFile {
    algorithm: String,
    value: String,
}

impl WorkflowRun {
    pub(crate) fn create(
        id: RunId,
        created_at_ms: u64,
        agent_id: AgentId,
        pinned: PinnedWorkflowDefinition,
        environments: ResolvedEnvironmentSet,
    ) -> Self {
        let step = pinned.definition.first_step().clone();
        Self {
            id,
            created_at_ms,
            agent_id,
            pinned,
            environments,
            state: RunState::Ready { step },
            source: RunSource::Pending,
            artefacts: Vec::new(),
            attempts: Vec::new(),
            gates: Vec::new(),
            transitions: Vec::new(),
        }
    }

    pub(crate) fn record_initial_candidate(
        &mut self,
        record: crate::workflows::artefacts::ArtefactRecord,
    ) -> Result<(), TransitionError> {
        if !matches!(self.source, RunSource::Pending) {
            return Err(TransitionError::Invalid);
        }
        if record.kind != crate::workflows::definition::ArtefactKind::CandidateRevision {
            return Err(TransitionError::Invalid);
        }
        if record.provenance.run_id != self.id {
            return Err(TransitionError::Invalid);
        }
        let reference = ArtefactReference {
            id: record.id,
            kind: record.kind,
            artefact_hash: record.artefact_hash,
        };
        self.artefacts.push(record);
        self.source = RunSource::Captured {
            source: RunSourceState {
                initial: reference.clone(),
                accepted: reference.clone(),
                observed: ObservedCandidate::Exact {
                    artefact: reference,
                },
            },
        };
        Ok(())
    }

    pub(crate) fn artefact(
        &self,
        id: &crate::workflows::id::ArtefactId,
    ) -> Option<&crate::workflows::artefacts::ArtefactRecord> {
        self.artefacts.iter().find(|record| record.id == *id)
    }

    pub(crate) fn observed_candidate_hash(
        &self,
    ) -> Option<crate::workflows::artefacts::CandidateHash> {
        let RunSource::Captured { source } = &self.source else {
            return None;
        };
        let ObservedCandidate::Exact { artefact } = &source.observed else {
            return None;
        };
        self.artefact(&artefact.id)
            .and_then(crate::workflows::artefacts::ArtefactRecord::candidate_hash)
    }

    pub(crate) fn open_gate(
        &mut self,
        gate_id: GateId,
        candidate: ArtefactReference,
        diff_base: ArtefactReference,
        at_ms: u64,
    ) -> Result<HumanGateRecord, TransitionError> {
        if !self.accepts_time(at_ms) || self.gates.iter().any(|gate| gate.id == gate_id) {
            return Err(TransitionError::Invalid);
        }
        let RunState::Ready { step } = self.state.clone() else {
            return Err(TransitionError::Invalid);
        };
        let definition = self
            .pinned
            .definition
            .step(&step)
            .ok_or(TransitionError::Invalid)?;
        let StepAction::HumanGate(action) = &definition.action else {
            return Err(TransitionError::Invalid);
        };
        if candidate.kind != super::definition::ArtefactKind::CandidateRevision
            || diff_base.kind != super::definition::ArtefactKind::CandidateRevision
        {
            return Err(TransitionError::Invalid);
        }
        let sequence = u32::try_from(self.gates.len() + 1).map_err(|_| TransitionError::Invalid)?;
        let transition_sequence = self.transitions.last().map_or(1, |item| item.sequence + 1);
        let revision = GateRevision::new(transition_sequence).ok_or(TransitionError::Invalid)?;
        let gate = HumanGateRecord {
            id: gate_id,
            step: step.clone(),
            sequence,
            revision,
            opened_at_ms: at_ms,
            closed_at_ms: None,
            candidate,
            diff_base,
            state: HumanGateState::AwaitingDecision,
            decision: None,
            output: action.required_output.key.clone(),
        };
        self.gates.push(gate.clone());
        let from = self.state.clone();
        self.push_transition(
            at_ms,
            TransitionCause::GateOpened,
            from,
            RunState::AwaitingHuman {
                step,
                gate: gate_id,
            },
        );
        Ok(gate)
    }

    pub(crate) fn decide_gate(
        &mut self,
        gate_id: GateId,
        revision: GateRevision,
        record: ArtefactRecord,
        kind: super::gates::HumanDecisionKind,
        at_ms: u64,
    ) -> Result<(), TransitionError> {
        if !self.accepts_time(at_ms) || record.provenance.run_id != self.id {
            return Err(TransitionError::Invalid);
        }
        let RunState::AwaitingHuman { step, gate } = self.state.clone() else {
            return Err(TransitionError::Invalid);
        };
        if gate != gate_id || record.kind != super::definition::ArtefactKind::HumanDecision {
            return Err(TransitionError::Invalid);
        }
        let gate = self
            .gates
            .iter_mut()
            .find(|item| item.id == gate_id)
            .ok_or(TransitionError::Invalid)?;
        if gate.state != HumanGateState::AwaitingDecision || gate.revision != revision {
            return Err(TransitionError::Invalid);
        }
        let reference = ArtefactReference {
            id: record.id,
            kind: record.kind,
            artefact_hash: record.artefact_hash,
        };
        gate.closed_at_ms = Some(at_ms);
        gate.decision = Some(reference.clone());
        gate.state = match kind {
            super::gates::HumanDecisionKind::Approved => HumanGateState::Approved,
            super::gates::HumanDecisionKind::RevisionRequested => HumanGateState::RevisionRequested,
        };
        self.artefacts.push(record);
        let from = self.state.clone();
        match kind {
            super::gates::HumanDecisionKind::Approved => {
                let definition = self
                    .pinned
                    .definition
                    .step(&step)
                    .ok_or(TransitionError::Invalid)?;
                let to = match &definition.on_success {
                    SuccessTransition::Next(next) => RunState::Ready { step: next.clone() },
                    SuccessTransition::CompleteRun => RunState::Completed,
                };
                self.push_transition(at_ms, TransitionCause::GateApproved, from, to);
            }
            super::gates::HumanDecisionKind::RevisionRequested => self.push_transition(
                at_ms,
                TransitionCause::GateRevisionRequested,
                from,
                RunState::RevisionRequested {
                    step,
                    decision: reference,
                },
            ),
        }
        Ok(())
    }

    pub(crate) fn cancel_gate(
        &mut self,
        gate_id: GateId,
        revision: GateRevision,
        at_ms: u64,
    ) -> Result<(), TransitionError> {
        if !self.accepts_time(at_ms) {
            return Err(TransitionError::Invalid);
        }
        let RunState::AwaitingHuman { gate, .. } = self.state else {
            return Err(TransitionError::Invalid);
        };
        if gate != gate_id {
            return Err(TransitionError::Invalid);
        }
        let record = self
            .gates
            .iter_mut()
            .find(|item| item.id == gate_id)
            .ok_or(TransitionError::Invalid)?;
        if record.state != HumanGateState::AwaitingDecision || record.revision != revision {
            return Err(TransitionError::Invalid);
        }
        record.state = HumanGateState::Cancelled;
        record.closed_at_ms = Some(at_ms);
        let from = self.state.clone();
        self.push_transition(
            at_ms,
            TransitionCause::CancellationRequested,
            from,
            RunState::Cancelled,
        );
        Ok(())
    }

    pub(crate) fn start_attempt(
        &mut self,
        attempt_id: AttemptId,
        inputs: Vec<AttemptArtefactInput>,
        capabilities: AttemptCapabilities,
        sandbox: AttemptSandboxRecord,
        at_ms: u64,
    ) -> Result<(), TransitionError> {
        if !self.accepts_time(at_ms) {
            return Err(TransitionError::Invalid);
        }
        let RunState::Ready { step } = self.state.clone() else {
            return Err(TransitionError::Invalid);
        };
        if self.attempts.iter().any(|attempt| attempt.id == attempt_id) {
            return Err(TransitionError::Invalid);
        }
        if self
            .attempts
            .iter()
            .any(|attempt| attempt.state == AttemptState::Active)
        {
            return Err(TransitionError::Invalid);
        }
        let Some(definition_step) = self.pinned.definition.step(&step) else {
            return Err(TransitionError::Invalid);
        };
        if !capabilities_match_step(&capabilities, definition_step) {
            return Err(TransitionError::Invalid);
        }
        let expected = self
            .environments
            .steps
            .iter()
            .find(|item| item.step == step)
            .map(|item| &item.snapshot_digest);
        if expected != Some(&sandbox.snapshot_digest)
            || sandbox.kind != AttemptSandboxKind::IsolatedAttempt
        {
            return Err(TransitionError::Invalid);
        }
        let ordinal = next_ordinal(&self.attempts, &step);
        self.attempts.push(AttemptRecord {
            id: attempt_id,
            step: step.clone(),
            ordinal,
            action_kind: ActionKind::from_action(&definition_step.action),
            started_at_ms: at_ms,
            finished_at_ms: None,
            state: AttemptState::Active,
            result: None,
            inputs,
            outputs: Vec::new(),
            capabilities,
            sandbox,
            cleanup: AttemptCleanupRecord::Pending,
            commit_transaction: None,
            commit_result: None,
        });
        let from = self.state.clone();
        let to = RunState::Active {
            step,
            attempt: attempt_id,
        };
        self.push_transition(at_ms, TransitionCause::AttemptStarted, from, to);
        Ok(())
    }

    pub(crate) fn record_commit_transaction(
        &mut self,
        attempt_id: AttemptId,
        transaction: CommitTransaction,
    ) -> Result<(), TransitionError> {
        let Some(attempt) = self
            .attempts
            .iter_mut()
            .find(|attempt| attempt.id == attempt_id && attempt.state == AttemptState::Active)
        else {
            return Err(TransitionError::Invalid);
        };
        if let Some(current) = &attempt.commit_transaction
            && !current.can_advance_to(&transaction)
        {
            return Err(TransitionError::Invalid);
        }
        attempt.commit_transaction = Some(transaction);
        Ok(())
    }

    pub(crate) fn record_commit_result(
        &mut self,
        attempt_id: AttemptId,
        result: CommitResult,
    ) -> Result<(), TransitionError> {
        let Some(attempt) = self
            .attempts
            .iter_mut()
            .find(|attempt| attempt.id == attempt_id && attempt.state == AttemptState::Active)
        else {
            return Err(TransitionError::Invalid);
        };
        if let Some(current) = &attempt.commit_result {
            return if current == &result {
                Ok(())
            } else {
                Err(TransitionError::Invalid)
            };
        }
        let Some(transaction) = &attempt.commit_transaction else {
            return Err(TransitionError::Invalid);
        };
        if transaction.verified_commit() != Some(result.commit.as_str()) {
            return Err(TransitionError::Invalid);
        }
        attempt.commit_result = Some(result);
        Ok(())
    }

    pub(crate) fn record_cleanup(
        &mut self,
        attempt_id: AttemptId,
        cleanup: AttemptCleanupRecord,
    ) -> Result<(), TransitionError> {
        let Some(attempt) = self
            .attempts
            .iter_mut()
            .find(|attempt| attempt.id == attempt_id)
        else {
            return Err(TransitionError::Invalid);
        };
        if attempt.cleanup == cleanup && !matches!(cleanup, AttemptCleanupRecord::Pending) {
            return Ok(());
        }
        if !matches!(attempt.cleanup, AttemptCleanupRecord::Pending)
            || matches!(cleanup, AttemptCleanupRecord::Pending)
        {
            return Err(TransitionError::Invalid);
        }
        attempt.cleanup = cleanup;
        Ok(())
    }

    pub(crate) fn record_attempt_outputs(
        &mut self,
        attempt_id: AttemptId,
        artefacts: Vec<crate::workflows::artefacts::ArtefactRecord>,
        outputs: Vec<AttemptArtefactOutput>,
        accepted: Option<ArtefactReference>,
        observed: ObservedCandidate,
    ) -> Result<(), TransitionError> {
        if artefacts.len()
            > crate::workflows::artefacts::MAXIMUM_ARTEFACTS.saturating_sub(self.artefacts.len())
        {
            return Err(TransitionError::Invalid);
        }
        for record in &artefacts {
            if record.provenance.run_id != self.id
                || self.artefacts.iter().any(|item| item.id == record.id)
            {
                return Err(TransitionError::Invalid);
            }
        }
        let Some(attempt) = self
            .attempts
            .iter_mut()
            .find(|item| item.id == attempt_id && item.state == AttemptState::Active)
        else {
            return Err(TransitionError::Invalid);
        };
        attempt.outputs = outputs;
        self.artefacts.extend(artefacts);
        if let RunSource::Captured { source } = &mut self.source {
            source.observed = observed;
            if let Some(accepted) = accepted {
                source.accepted = accepted;
            }
        }
        Ok(())
    }

    pub(crate) fn complete_attempt(
        &mut self,
        attempt_id: AttemptId,
        at_ms: u64,
    ) -> Result<(), TransitionError> {
        if !self.accepts_time(at_ms) {
            return Err(TransitionError::Invalid);
        }
        let RunState::Active { step, attempt } = self.state.clone() else {
            return Err(TransitionError::Invalid);
        };
        if attempt != attempt_id {
            return Err(TransitionError::Invalid);
        }
        let definition_step = self
            .pinned
            .definition
            .step(&step)
            .ok_or(TransitionError::Invalid)?;
        let on_success = definition_step.on_success.clone();
        let expected_outputs = required_output_keys(&definition_step.action);
        let Some(attempt) = self.attempts.iter().find(|item| item.id == attempt_id) else {
            return Err(TransitionError::Invalid);
        };
        if attempt.cleanup != AttemptCleanupRecord::Complete
            || !durable_outputs_match(attempt, definition_step)
        {
            return Err(TransitionError::Invalid);
        }
        self.finish_attempt(
            attempt_id,
            at_ms,
            AttemptState::Completed,
            AttemptResult::Completed {
                outputs: expected_outputs,
            },
        )?;
        let from = self.state.clone();
        let to = match on_success {
            SuccessTransition::Next(next) => RunState::Ready { step: next },
            SuccessTransition::CompleteRun => RunState::Completed,
        };
        self.push_transition(at_ms, TransitionCause::AttemptCompleted, from, to);
        Ok(())
    }

    pub(crate) fn fail_before_attempt(&mut self, at_ms: u64) -> Result<(), TransitionError> {
        if !self.accepts_time(at_ms) {
            return Err(TransitionError::Invalid);
        }
        match self.state.clone() {
            RunState::Ready { .. } | RunState::InitialisingSource => {
                let from = self.state.clone();
                self.push_transition(
                    at_ms,
                    TransitionCause::InitialSourceCaptureFailed,
                    from,
                    RunState::Failed,
                );
                Ok(())
            }
            _ => Err(TransitionError::Invalid),
        }
    }

    pub(crate) fn fail_attempt(
        &mut self,
        attempt_id: AttemptId,
        category: FailureCategory,
        at_ms: u64,
    ) -> Result<(), TransitionError> {
        if !self.accepts_time(at_ms) {
            return Err(TransitionError::Invalid);
        }
        let RunState::Active { attempt, .. } = self.state.clone() else {
            return Err(TransitionError::Invalid);
        };
        if attempt != attempt_id {
            return Err(TransitionError::Invalid);
        }
        let Some(record) = self.attempts.iter().find(|item| item.id == attempt_id) else {
            return Err(TransitionError::Invalid);
        };
        if matches!(record.cleanup, AttemptCleanupRecord::Pending) {
            return Err(TransitionError::Invalid);
        }
        self.finish_attempt(
            attempt_id,
            at_ms,
            AttemptState::Failed,
            AttemptResult::Failed { category },
        )?;
        let from = self.state.clone();
        self.push_transition(
            at_ms,
            TransitionCause::AttemptFailed,
            from,
            RunState::Failed,
        );
        Ok(())
    }

    pub(crate) fn cancel(&mut self, at_ms: u64) -> Result<(), TransitionError> {
        if !self.accepts_time(at_ms) {
            return Err(TransitionError::Invalid);
        }
        match self.state.clone() {
            RunState::Ready { .. } => {
                let from = self.state.clone();
                self.push_transition(
                    at_ms,
                    TransitionCause::CancellationRequested,
                    from,
                    RunState::Cancelled,
                );
                Ok(())
            }
            RunState::Active { attempt, .. } => {
                let Some(record) = self.attempts.iter().find(|item| item.id == attempt) else {
                    return Err(TransitionError::Invalid);
                };
                if matches!(record.cleanup, AttemptCleanupRecord::Pending) {
                    return Err(TransitionError::Invalid);
                }
                self.finish_attempt(
                    attempt,
                    at_ms,
                    AttemptState::Cancelled,
                    AttemptResult::Cancelled,
                )?;
                let from = self.state.clone();
                self.push_transition(
                    at_ms,
                    TransitionCause::CancellationRequested,
                    from,
                    RunState::Cancelled,
                );
                Ok(())
            }
            RunState::InitialisingSource => {
                let from = self.state.clone();
                self.push_transition(
                    at_ms,
                    TransitionCause::CancellationRequested,
                    from,
                    RunState::Cancelled,
                );
                Ok(())
            }
            RunState::AwaitingHuman { gate, .. } => {
                let revision = self
                    .gates
                    .iter()
                    .find(|item| item.id == gate)
                    .map(|item| item.revision)
                    .ok_or(TransitionError::Invalid)?;
                self.cancel_gate(gate, revision, at_ms)
            }
            RunState::Completed
            | RunState::RevisionRequested { .. }
            | RunState::Failed
            | RunState::Cancelled
            | RunState::Interrupted => Err(TransitionError::Invalid),
        }
    }

    pub(crate) fn interrupt(&mut self, at_ms: u64) -> Result<(), TransitionError> {
        if !self.accepts_time(at_ms) {
            return Err(TransitionError::Invalid);
        }
        if matches!(self.state, RunState::InitialisingSource) {
            let from = self.state.clone();
            self.push_transition(
                at_ms,
                TransitionCause::ProcessRestarted,
                from,
                RunState::Interrupted,
            );
            return Ok(());
        }
        if let RunState::AwaitingHuman { gate, .. } = self.state.clone() {
            let record = self
                .gates
                .iter_mut()
                .find(|item| item.id == gate)
                .ok_or(TransitionError::Invalid)?;
            if record.state != HumanGateState::AwaitingDecision {
                return Err(TransitionError::Invalid);
            }
            record.state = HumanGateState::Interrupted;
            record.closed_at_ms = Some(at_ms);
            let from = self.state.clone();
            self.push_transition(
                at_ms,
                TransitionCause::ProcessRestarted,
                from,
                RunState::Interrupted,
            );
            return Ok(());
        }
        let RunState::Active { attempt, .. } = self.state.clone() else {
            return Err(TransitionError::Invalid);
        };
        self.finish_attempt(
            attempt,
            at_ms,
            AttemptState::Interrupted,
            AttemptResult::Interrupted,
        )?;
        let from = self.state.clone();
        self.push_transition(
            at_ms,
            TransitionCause::ProcessRestarted,
            from,
            RunState::Interrupted,
        );
        Ok(())
    }

    pub(crate) fn ready_step(&self) -> Option<&StepKey> {
        match &self.state {
            RunState::Ready { step } => Some(step),
            _ => None,
        }
    }

    pub(crate) fn active_attempt(&self) -> Option<AttemptId> {
        match self.state {
            RunState::Active { attempt, .. } => Some(attempt),
            _ => None,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        matches!(
            self.state,
            RunState::Active { .. } | RunState::AwaitingHuman { .. }
        )
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            RunState::Completed
                | RunState::RevisionRequested { .. }
                | RunState::Failed
                | RunState::Cancelled
                | RunState::Interrupted
        )
    }

    pub(crate) fn current_step_name(&self) -> Option<&str> {
        let key = match &self.state {
            RunState::Ready { step }
            | RunState::Active { step, .. }
            | RunState::AwaitingHuman { step, .. }
            | RunState::RevisionRequested { step, .. } => step,
            _ => self.attempts.last().map(|attempt| &attempt.step)?,
        };
        self.pinned
            .definition
            .step(key)
            .map(|step| step.name.as_str())
    }

    pub(crate) fn latest_attempt(&self) -> Option<&AttemptRecord> {
        self.attempts.last()
    }

    fn accepts_time(&self, at_ms: u64) -> bool {
        at_ms >= self.created_at_ms
            && self
                .transitions
                .last()
                .is_none_or(|transition| at_ms >= transition.occurred_at_ms)
    }

    fn finish_attempt(
        &mut self,
        attempt_id: AttemptId,
        at_ms: u64,
        state: AttemptState,
        result: AttemptResult,
    ) -> Result<(), TransitionError> {
        let Some(attempt) = self
            .attempts
            .iter_mut()
            .find(|attempt| attempt.id == attempt_id)
        else {
            return Err(TransitionError::Invalid);
        };
        if attempt.state != AttemptState::Active || attempt.result.is_some() {
            return Err(TransitionError::Invalid);
        }
        attempt.state = state;
        attempt.finished_at_ms = Some(at_ms);
        attempt.result = Some(result);
        Ok(())
    }

    fn push_transition(
        &mut self,
        occurred_at_ms: u64,
        cause: TransitionCause,
        from: RunState,
        to: RunState,
    ) {
        let sequence = self
            .transitions
            .last()
            .map(|transition| transition.sequence + 1)
            .unwrap_or(1);
        self.transitions.push(TransitionRecord {
            sequence,
            occurred_at_ms,
            cause,
            from,
            to: to.clone(),
        });
        self.state = to;
    }

    pub(super) fn to_file(&self) -> RunFile {
        RunFile {
            record_version: RUN_RECORD_VERSION,
            id: self.id.as_hex(),
            created_at_ms: self.created_at_ms,
            agent_id: self.agent_id.as_hex(),
            workflow_id: self.pinned.workflow_id.map(|id| id.as_hex()),
            version: self.pinned.version.as_hex(),
            definition: self.pinned.definition.to_file(),
            environments: environment_set_to_file(&self.environments),
            state: RunStateFile::from_state(&self.state),
            source: source_to_file(&self.source),
            artefacts: self.artefacts.iter().map(artefact_to_file).collect(),
            attempts: self.attempts.iter().map(AttemptRecord::to_file).collect(),
            gates: self.gates.iter().map(gate_to_file).collect(),
            transitions: self
                .transitions
                .iter()
                .map(TransitionRecord::to_file)
                .collect(),
        }
    }

    pub(super) fn from_file(file: RunFile) -> Result<Self, RunRecordError> {
        if file.record_version != RUN_RECORD_VERSION {
            return Err(RunRecordError::Corrupt);
        }
        let id = RunId::parse(&file.id).ok_or(RunRecordError::Corrupt)?;
        let agent_id = AgentId::parse(&file.agent_id).ok_or(RunRecordError::Corrupt)?;
        let workflow_id = match file.workflow_id {
            Some(value) => Some(WorkflowId::parse(&value).ok_or(RunRecordError::Corrupt)?),
            None => None,
        };
        let stored_version =
            DefinitionVersion::parse(&file.version).ok_or(RunRecordError::Corrupt)?;
        let definition =
            WorkflowDefinition::from_file(file.definition).map_err(|_| RunRecordError::Corrupt)?;
        let version = definition.version();
        if version != stored_version {
            return Err(RunRecordError::Corrupt);
        }
        let state = RunState::from_file(file.state)?;
        let attempts = file
            .attempts
            .into_iter()
            .map(AttemptRecord::from_file)
            .collect::<Result<Vec<_>, _>>()?;
        let gates = file
            .gates
            .into_iter()
            .map(gate_from_file)
            .collect::<Result<Vec<_>, _>>()?;
        let transitions = file
            .transitions
            .into_iter()
            .map(TransitionRecord::from_file)
            .collect::<Result<Vec<_>, _>>()?;
        let environments = environment_set_from_file(file.environments)?;
        let source = source_from_file(file.source)?;
        let artefacts = file
            .artefacts
            .into_iter()
            .map(artefact_from_file)
            .collect::<Result<Vec<_>, _>>()?;
        let run = Self {
            id,
            created_at_ms: file.created_at_ms,
            agent_id,
            pinned: PinnedWorkflowDefinition {
                workflow_id,
                version,
                definition,
            },
            environments,
            state,
            source,
            artefacts,
            attempts,
            gates,
            transitions,
        };
        run.validate_loaded()?;
        Ok(run)
    }

    fn validate_loaded(&self) -> Result<(), RunRecordError> {
        if self.attempts.len() > self.pinned.definition.steps().len()
            || self.transitions.len() > self.pinned.definition.steps().len() * 2 + 1
        {
            return Err(RunRecordError::Corrupt);
        }
        for (index, attempt) in self.attempts.iter().enumerate() {
            if self.attempts[..index]
                .iter()
                .any(|earlier| earlier.id == attempt.id)
            {
                return Err(RunRecordError::Corrupt);
            }
            let step = self
                .pinned
                .definition
                .step(&attempt.step)
                .ok_or(RunRecordError::Corrupt)?;
            if attempt.action_kind != ActionKind::from_action(&step.action)
                || attempt.ordinal != next_ordinal(&self.attempts[..index], &attempt.step)
                || attempt.started_at_ms < self.created_at_ms
            {
                return Err(RunRecordError::Corrupt);
            }
            validate_attempt_result(attempt, step)?;
            validate_attempt_isolation(attempt, step, self)?;
            validate_report_binding(attempt, step, self)?;
        }
        validate_gates(self)?;
        validate_transitions(
            self.created_at_ms,
            &self.pinned.definition,
            &self.attempts,
            &self.gates,
            &self.transitions,
            &self.state,
        )?;
        validate_environments(self)
    }
}

impl RunState {
    pub(crate) fn as_label(&self) -> &'static str {
        match self {
            Self::InitialisingSource => "Source capture",
            Self::Ready { .. } => "Ready",
            Self::Active { .. } => "Active",
            Self::AwaitingHuman { .. } => "Awaiting decision",
            Self::RevisionRequested { .. } => "Revision requested",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Interrupted => "Interrupted",
        }
    }

    fn from_file(file: RunStateFile) -> Result<Self, RunRecordError> {
        Ok(match file {
            RunStateFile::InitialisingSource => Self::InitialisingSource,
            RunStateFile::Ready { step } => Self::Ready {
                step: StepKey::parse(&step).map_err(|_| RunRecordError::Corrupt)?,
            },
            RunStateFile::Active { step, attempt } => Self::Active {
                step: StepKey::parse(&step).map_err(|_| RunRecordError::Corrupt)?,
                attempt: AttemptId::parse(&attempt).ok_or(RunRecordError::Corrupt)?,
            },
            RunStateFile::AwaitingHuman { step, gate } => Self::AwaitingHuman {
                step: StepKey::parse(&step).map_err(|_| RunRecordError::Corrupt)?,
                gate: GateId::parse(&gate).ok_or(RunRecordError::Corrupt)?,
            },
            RunStateFile::RevisionRequested { step, decision } => Self::RevisionRequested {
                step: StepKey::parse(&step).map_err(|_| RunRecordError::Corrupt)?,
                decision: ref_from_file(decision)?,
            },
            RunStateFile::Completed => Self::Completed,
            RunStateFile::Failed => Self::Failed,
            RunStateFile::Cancelled => Self::Cancelled,
            RunStateFile::Interrupted => Self::Interrupted,
        })
    }
}

impl RunStateFile {
    fn from_state(state: &RunState) -> Self {
        match state {
            RunState::InitialisingSource => Self::InitialisingSource,
            RunState::Ready { step } => Self::Ready {
                step: step.as_str().to_owned(),
            },
            RunState::Active { step, attempt } => Self::Active {
                step: step.as_str().to_owned(),
                attempt: attempt.as_hex(),
            },
            RunState::AwaitingHuman { step, gate } => Self::AwaitingHuman {
                step: step.as_str().to_owned(),
                gate: gate.as_hex(),
            },
            RunState::RevisionRequested { step, decision } => Self::RevisionRequested {
                step: step.as_str().to_owned(),
                decision: ref_to_file(decision),
            },
            RunState::Completed => Self::Completed,
            RunState::Failed => Self::Failed,
            RunState::Cancelled => Self::Cancelled,
            RunState::Interrupted => Self::Interrupted,
        }
    }
}

impl AttemptRecord {
    fn to_file(&self) -> AttemptFile {
        AttemptFile {
            id: self.id.as_hex(),
            step: self.step.as_str().to_owned(),
            ordinal: self.ordinal,
            action_kind: self.action_kind.as_str().to_owned(),
            started_at_ms: self.started_at_ms,
            finished_at_ms: self.finished_at_ms,
            state: self.state.as_str().to_owned(),
            result: self.result.as_ref().map(AttemptResult::to_file),
            inputs: self
                .inputs
                .iter()
                .map(|input| AttemptInputFile {
                    key: input.key.as_str().to_owned(),
                    artefact: ref_to_file(&input.artefact),
                })
                .collect(),
            outputs: self
                .outputs
                .iter()
                .map(|output| AttemptOutputFile {
                    key: output.key.as_str().to_owned(),
                    artefact: ref_to_file(&output.artefact),
                })
                .collect(),
            capabilities: capabilities_to_file(&self.capabilities),
            sandbox: AttemptSandboxFile {
                kind: self.sandbox.kind.as_str().to_owned(),
                snapshot_digest: self.sandbox.snapshot_digest.as_str().to_owned(),
            },
            cleanup: cleanup_to_file(&self.cleanup),
            commit_transaction: self
                .commit_transaction
                .as_ref()
                .map(commit_transaction_to_file),
            commit_result: self.commit_result.as_ref().map(|result| CommitResultFile {
                commit: result.commit.clone(),
            }),
        }
    }

    fn from_file(file: AttemptFile) -> Result<Self, RunRecordError> {
        if file.ordinal == 0 {
            return Err(RunRecordError::Corrupt);
        }
        Ok(Self {
            id: AttemptId::parse(&file.id).ok_or(RunRecordError::Corrupt)?,
            step: StepKey::parse(&file.step).map_err(|_| RunRecordError::Corrupt)?,
            ordinal: file.ordinal,
            action_kind: ActionKind::parse(&file.action_kind).ok_or(RunRecordError::Corrupt)?,
            started_at_ms: file.started_at_ms,
            finished_at_ms: file.finished_at_ms,
            state: AttemptState::parse(&file.state).ok_or(RunRecordError::Corrupt)?,
            result: file.result.map(AttemptResult::from_file).transpose()?,
            inputs: file
                .inputs
                .into_iter()
                .map(input_from_file)
                .collect::<Result<Vec<_>, _>>()?,
            outputs: file
                .outputs
                .into_iter()
                .map(output_from_file)
                .collect::<Result<Vec<_>, _>>()?,
            capabilities: capabilities_from_file(file.capabilities)?,
            sandbox: AttemptSandboxRecord {
                kind: AttemptSandboxKind::parse(&file.sandbox.kind)
                    .ok_or(RunRecordError::Corrupt)?,
                snapshot_digest: SnapshotDigest::parse(&file.sandbox.snapshot_digest)
                    .ok_or(RunRecordError::Corrupt)?,
            },
            cleanup: cleanup_from_file(file.cleanup)?,
            commit_transaction: file
                .commit_transaction
                .map(commit_transaction_from_file)
                .transpose()?,
            commit_result: file.commit_result.map(|result| CommitResult {
                commit: result.commit,
            }),
        })
    }
}

impl AttemptState {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Interrupted => "Interrupted",
        }
    }
}

impl AttemptResult {
    fn to_file(&self) -> AttemptResultFile {
        match self {
            Self::Completed { outputs } => AttemptResultFile::Completed {
                outputs: outputs.clone(),
            },
            Self::Failed { category } => AttemptResultFile::Failed {
                category: category.as_str().to_owned(),
            },
            Self::Cancelled => AttemptResultFile::Cancelled,
            Self::Interrupted => AttemptResultFile::Interrupted,
        }
    }

    fn from_file(file: AttemptResultFile) -> Result<Self, RunRecordError> {
        Ok(match file {
            AttemptResultFile::Completed { outputs } => Self::Completed { outputs },
            AttemptResultFile::Failed { category } => Self::Failed {
                category: FailureCategory::parse(&category).ok_or(RunRecordError::Corrupt)?,
            },
            AttemptResultFile::Cancelled => Self::Cancelled,
            AttemptResultFile::Interrupted => Self::Interrupted,
        })
    }

    pub(crate) fn as_label(&self) -> String {
        match self {
            Self::Completed { .. } => "Completed".to_owned(),
            Self::Failed { category } => format!("Failed ({})", category.as_label()),
            Self::Cancelled => "Cancelled".to_owned(),
            Self::Interrupted => "Interrupted".to_owned(),
        }
    }
}

impl FailureCategory {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "provider" => Some(Self::Provider),
            "tool" => Some(Self::Tool),
            "authority" => Some(Self::Authority),
            "command" => Some(Self::Command),
            "operational" => Some(Self::Operational),
            "definition" => Some(Self::Definition),
            "cleanup" => Some(Self::Cleanup),
            "assurance" => Some(Self::Assurance),
            "commit" => Some(Self::Commit),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Tool => "tool",
            Self::Authority => "authority",
            Self::Command => "command",
            Self::Operational => "operational",
            Self::Definition => "definition",
            Self::Cleanup => "cleanup",
            Self::Assurance => "assurance",
            Self::Commit => "commit",
        }
    }

    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Tool => "tool",
            Self::Authority => "authority",
            Self::Command => "command",
            Self::Operational => "operational",
            Self::Definition => "definition",
            Self::Cleanup => "cleanup",
            Self::Assurance => "assurance",
            Self::Commit => "commit",
        }
    }
}

impl ActionKind {
    fn from_action(action: &StepAction) -> Self {
        match action {
            StepAction::Agent(_) => Self::Agent,
            StepAction::SystemCommand(_) => Self::SystemCommand,
            StepAction::HumanGate(_) => Self::HumanGate,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "agent" => Some(Self::Agent),
            "system-command" => Some(Self::SystemCommand),
            "human-gate" => Some(Self::HumanGate),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::SystemCommand => "system-command",
            Self::HumanGate => "human-gate",
        }
    }

    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::SystemCommand => "System command",
            Self::HumanGate => "Human gate",
        }
    }
}

impl TransitionRecord {
    fn to_file(&self) -> TransitionFile {
        TransitionFile {
            sequence: self.sequence,
            occurred_at_ms: self.occurred_at_ms,
            cause: self.cause.as_str().to_owned(),
            from: RunStateFile::from_state(&self.from),
            to: RunStateFile::from_state(&self.to),
        }
    }

    fn from_file(file: TransitionFile) -> Result<Self, RunRecordError> {
        Ok(Self {
            sequence: file.sequence,
            occurred_at_ms: file.occurred_at_ms,
            cause: TransitionCause::parse(&file.cause).ok_or(RunRecordError::Corrupt)?,
            from: RunState::from_file(file.from)?,
            to: RunState::from_file(file.to)?,
        })
    }
}

impl TransitionCause {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "initial-source-capture-started" => Some(Self::InitialSourceCaptureStarted),
            "initial-source-captured" => Some(Self::InitialSourceCaptured),
            "initial-source-capture-failed" => Some(Self::InitialSourceCaptureFailed),
            "source-drift-detected" => Some(Self::SourceDriftDetected),
            "attempt-started" => Some(Self::AttemptStarted),
            "attempt-completed" => Some(Self::AttemptCompleted),
            "attempt-failed" => Some(Self::AttemptFailed),
            "cancellation-requested" => Some(Self::CancellationRequested),
            "process-restarted" => Some(Self::ProcessRestarted),
            "gate-opened" => Some(Self::GateOpened),
            "gate-approved" => Some(Self::GateApproved),
            "gate-revision-requested" => Some(Self::GateRevisionRequested),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::InitialSourceCaptureStarted => "initial-source-capture-started",
            Self::InitialSourceCaptured => "initial-source-captured",
            Self::InitialSourceCaptureFailed => "initial-source-capture-failed",
            Self::SourceDriftDetected => "source-drift-detected",
            Self::AttemptStarted => "attempt-started",
            Self::AttemptCompleted => "attempt-completed",
            Self::AttemptFailed => "attempt-failed",
            Self::CancellationRequested => "cancellation-requested",
            Self::ProcessRestarted => "process-restarted",
            Self::GateOpened => "gate-opened",
            Self::GateApproved => "gate-approved",
            Self::GateRevisionRequested => "gate-revision-requested",
        }
    }
}

fn validate_gates(run: &WorkflowRun) -> Result<(), RunRecordError> {
    for (index, gate) in run.gates.iter().enumerate() {
        if gate.sequence != u32::try_from(index + 1).map_err(|_| RunRecordError::Corrupt)?
            || gate.candidate.kind != crate::workflows::definition::ArtefactKind::CandidateRevision
            || gate.diff_base.kind != crate::workflows::definition::ArtefactKind::CandidateRevision
            || !run
                .artefacts
                .iter()
                .any(|record| record.id == gate.candidate.id)
            || !run
                .artefacts
                .iter()
                .any(|record| record.id == gate.diff_base.id)
        {
            return Err(RunRecordError::Corrupt);
        }
        let step = run
            .pinned
            .definition
            .step(&gate.step)
            .ok_or(RunRecordError::Corrupt)?;
        let StepAction::HumanGate(action) = &step.action else {
            return Err(RunRecordError::Corrupt);
        };
        if gate.output != action.required_output.key {
            return Err(RunRecordError::Corrupt);
        }
        match gate.state {
            HumanGateState::AwaitingDecision => {
                if gate.closed_at_ms.is_some() || gate.decision.is_some() {
                    return Err(RunRecordError::Corrupt);
                }
            }
            HumanGateState::Approved | HumanGateState::RevisionRequested => {
                let decision = gate.decision.as_ref().ok_or(RunRecordError::Corrupt)?;
                let record = run.artefact(&decision.id).ok_or(RunRecordError::Corrupt)?;
                let crate::workflows::artefacts::ArtefactProducer::HumanGate {
                    gate_id,
                    step,
                    output,
                } = &record.provenance.producer
                else {
                    return Err(RunRecordError::Corrupt);
                };
                if *gate_id != gate.id
                    || *step != gate.step
                    || *output != gate.output
                    || gate.closed_at_ms.is_none()
                    || decision.kind != crate::workflows::definition::ArtefactKind::HumanDecision
                {
                    return Err(RunRecordError::Corrupt);
                }
                let expected = if gate.state == HumanGateState::Approved {
                    super::gates::HumanDecisionKind::Approved
                } else {
                    super::gates::HumanDecisionKind::RevisionRequested
                };
                if !matches!(&record.summary, crate::workflows::artefacts::ArtefactSummary::HumanDecision { decision, .. } if *decision == expected)
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
            HumanGateState::Cancelled | HumanGateState::Interrupted => {
                if gate.closed_at_ms.is_none() || gate.decision.is_some() {
                    return Err(RunRecordError::Corrupt);
                }
            }
        }
    }
    Ok(())
}

fn validate_environments(run: &WorkflowRun) -> Result<(), RunRecordError> {
    let set = &run.environments;
    let mut seen = Vec::new();
    for environment in &set.environments {
        if seen.contains(&environment.environment_id) {
            return Err(RunRecordError::Corrupt);
        }
        seen.push(environment.environment_id);
    }
    let mut bound_steps = Vec::new();
    for binding in &set.steps {
        if bound_steps.contains(&binding.step) {
            return Err(RunRecordError::Corrupt);
        }
        let stored = set
            .environments
            .iter()
            .find(|environment| environment.environment_id == binding.environment_id)
            .ok_or(RunRecordError::Corrupt)?;
        if stored.preparation_id != binding.preparation_id
            || stored.snapshot.snapshot_digest != binding.snapshot_digest
        {
            return Err(RunRecordError::Corrupt);
        }
        let step = run
            .pinned
            .definition
            .step(&binding.step)
            .ok_or(RunRecordError::Corrupt)?;
        if !step.is_sandbox_backed() {
            return Err(RunRecordError::Corrupt);
        }
        bound_steps.push(binding.step.clone());
    }
    for step in run.pinned.definition.steps() {
        if step.is_sandbox_backed() && !bound_steps.iter().any(|key| key == &step.key) {
            return Err(RunRecordError::Corrupt);
        }
    }
    Ok(())
}

fn environment_set_to_file(set: &ResolvedEnvironmentSet) -> ResolvedEnvironmentSetFile {
    ResolvedEnvironmentSetFile {
        environments: set
            .environments
            .iter()
            .map(|environment| ResolvedEnvironmentFile {
                environment_id: environment.environment_id.as_hex(),
                name: environment.name.clone(),
                preparation_id: environment.preparation_id.as_hex(),
                recipe_version: environment.recipe_version.as_hex(),
                snapshot: PinnedSnapshotFile {
                    artifact_key: environment.snapshot.artifact_key.as_str().to_owned(),
                    snapshot_digest: environment.snapshot.snapshot_digest.as_str().to_owned(),
                    image_reference: environment.snapshot.image_reference.clone(),
                    image_manifest_digest: environment
                        .snapshot
                        .image_manifest_digest
                        .as_str()
                        .to_owned(),
                    upper_integrity: PinnedIntegrityFile {
                        algorithm: environment.snapshot.upper_integrity.algorithm.clone(),
                        value: environment.snapshot.upper_integrity.value.clone(),
                    },
                    upper_size_bytes: environment.snapshot.upper_size_bytes,
                },
            })
            .collect(),
        steps: set
            .steps
            .iter()
            .map(|binding| ResolvedStepEnvironmentFile {
                step: binding.step.as_str().to_owned(),
                environment_id: binding.environment_id.as_hex(),
                preparation_id: binding.preparation_id.as_hex(),
                snapshot_digest: binding.snapshot_digest.as_str().to_owned(),
            })
            .collect(),
    }
}

fn environment_set_from_file(
    file: ResolvedEnvironmentSetFile,
) -> Result<ResolvedEnvironmentSet, RunRecordError> {
    Ok(ResolvedEnvironmentSet {
        environments: file
            .environments
            .into_iter()
            .map(|environment| {
                Ok(ResolvedEnvironment {
                    environment_id: EnvironmentId::parse(&environment.environment_id)
                        .ok_or(RunRecordError::Corrupt)?,
                    name: environment.name,
                    preparation_id: PreparationId::parse(&environment.preparation_id)
                        .ok_or(RunRecordError::Corrupt)?,
                    recipe_version: EnvironmentRecipeVersion::parse(&environment.recipe_version)
                        .ok_or(RunRecordError::Corrupt)?,
                    snapshot: PreparedSnapshot {
                        artifact_key: SnapshotArtifactKey::parse(
                            &environment.snapshot.artifact_key,
                        )
                        .ok_or(RunRecordError::Corrupt)?,
                        snapshot_digest: SnapshotDigest::parse(
                            &environment.snapshot.snapshot_digest,
                        )
                        .ok_or(RunRecordError::Corrupt)?,
                        image_reference: environment.snapshot.image_reference,
                        image_manifest_digest: OciManifestDigest::parse(
                            &environment.snapshot.image_manifest_digest,
                        )
                        .ok_or(RunRecordError::Corrupt)?,
                        upper_integrity: RecordedIntegrity {
                            algorithm: environment.snapshot.upper_integrity.algorithm,
                            value: environment.snapshot.upper_integrity.value,
                        },
                        upper_size_bytes: environment.snapshot.upper_size_bytes,
                    },
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        steps: file
            .steps
            .into_iter()
            .map(|binding| {
                Ok(ResolvedStepEnvironment {
                    step: StepKey::parse(&binding.step).map_err(|_| RunRecordError::Corrupt)?,
                    environment_id: EnvironmentId::parse(&binding.environment_id)
                        .ok_or(RunRecordError::Corrupt)?,
                    preparation_id: PreparationId::parse(&binding.preparation_id)
                        .ok_or(RunRecordError::Corrupt)?,
                    snapshot_digest: SnapshotDigest::parse(&binding.snapshot_digest)
                        .ok_or(RunRecordError::Corrupt)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn ref_to_file(reference: &ArtefactReference) -> ArtefactRefFile {
    ArtefactRefFile {
        id: reference.id.as_hex(),
        kind: reference.kind.as_str().to_owned(),
        artefact_hash: reference.artefact_hash.as_str(),
    }
}

fn ref_from_file(file: ArtefactRefFile) -> Result<ArtefactReference, RunRecordError> {
    Ok(ArtefactReference {
        id: super::id::ArtefactId::parse(&file.id).ok_or(RunRecordError::Corrupt)?,
        kind: crate::workflows::definition::ArtefactKind::parse(&file.kind)
            .ok_or(RunRecordError::Corrupt)?,
        artefact_hash: crate::workflows::artefacts::ArtefactHash::parse(&file.artefact_hash)
            .ok_or(RunRecordError::Corrupt)?,
    })
}

fn gate_to_file(gate: &HumanGateRecord) -> HumanGateFile {
    HumanGateFile {
        id: gate.id.as_hex(),
        step: gate.step.as_str().to_owned(),
        sequence: gate.sequence,
        revision: gate.revision.get(),
        opened_at_ms: gate.opened_at_ms,
        closed_at_ms: gate.closed_at_ms,
        candidate: ref_to_file(&gate.candidate),
        diff_base: ref_to_file(&gate.diff_base),
        state: match gate.state {
            HumanGateState::AwaitingDecision => "awaiting-decision",
            HumanGateState::Approved => "approved",
            HumanGateState::RevisionRequested => "revision-requested",
            HumanGateState::Cancelled => "cancelled",
            HumanGateState::Interrupted => "interrupted",
        }
        .to_owned(),
        decision: gate.decision.as_ref().map(ref_to_file),
        output: gate.output.as_str().to_owned(),
    }
}

fn gate_from_file(file: HumanGateFile) -> Result<HumanGateRecord, RunRecordError> {
    Ok(HumanGateRecord {
        id: GateId::parse(&file.id).ok_or(RunRecordError::Corrupt)?,
        step: StepKey::parse(&file.step).map_err(|_| RunRecordError::Corrupt)?,
        sequence: file.sequence,
        revision: GateRevision::new(file.revision).ok_or(RunRecordError::Corrupt)?,
        opened_at_ms: file.opened_at_ms,
        closed_at_ms: file.closed_at_ms,
        candidate: ref_from_file(file.candidate)?,
        diff_base: ref_from_file(file.diff_base)?,
        state: match file.state.as_str() {
            "awaiting-decision" => HumanGateState::AwaitingDecision,
            "approved" => HumanGateState::Approved,
            "revision-requested" => HumanGateState::RevisionRequested,
            "cancelled" => HumanGateState::Cancelled,
            "interrupted" => HumanGateState::Interrupted,
            _ => return Err(RunRecordError::Corrupt),
        },
        decision: file.decision.map(ref_from_file).transpose()?,
        output: OutputKey::parse(&file.output).map_err(|_| RunRecordError::Corrupt)?,
    })
}

fn input_from_file(file: AttemptInputFile) -> Result<AttemptArtefactInput, RunRecordError> {
    Ok(AttemptArtefactInput {
        key: InputKey::parse(&file.key).map_err(|_| RunRecordError::Corrupt)?,
        artefact: ref_from_file(file.artefact)?,
    })
}

fn output_from_file(file: AttemptOutputFile) -> Result<AttemptArtefactOutput, RunRecordError> {
    Ok(AttemptArtefactOutput {
        key: OutputKey::parse(&file.key).map_err(|_| RunRecordError::Corrupt)?,
        artefact: ref_from_file(file.artefact)?,
    })
}

fn source_to_file(source: &RunSource) -> RunSourceFile {
    match source {
        RunSource::Pending => RunSourceFile::Pending,
        RunSource::Captured { source } => RunSourceFile::Captured {
            source: RunSourceStateFile {
                initial: ref_to_file(&source.initial),
                accepted: ref_to_file(&source.accepted),
                observed: match &source.observed {
                    ObservedCandidate::Exact { artefact } => ObservedCandidateFile::Exact {
                        artefact: ref_to_file(artefact),
                    },
                    ObservedCandidate::Unknown => ObservedCandidateFile::Unknown,
                },
            },
        },
    }
}

fn source_from_file(file: RunSourceFile) -> Result<RunSource, RunRecordError> {
    Ok(match file {
        RunSourceFile::Pending => RunSource::Pending,
        RunSourceFile::Captured { source } => RunSource::Captured {
            source: RunSourceState {
                initial: ref_from_file(source.initial)?,
                accepted: ref_from_file(source.accepted)?,
                observed: match source.observed {
                    ObservedCandidateFile::Exact { artefact } => ObservedCandidate::Exact {
                        artefact: ref_from_file(artefact)?,
                    },
                    ObservedCandidateFile::Unknown => ObservedCandidate::Unknown,
                },
            },
        },
    })
}

fn artefact_to_file(record: &ArtefactRecord) -> ArtefactFile {
    ArtefactFile {
        id: record.id.as_hex(),
        kind: record.kind.as_str().to_owned(),
        artefact_hash: record.artefact_hash.as_str(),
        object_hash: record.object_hash.as_str(),
        payload_bytes: record.payload_bytes,
        created_at_ms: record.created_at_ms,
        provenance: ProvenanceFile {
            run_id: record.provenance.run_id.as_hex(),
            producer: producer_to_file(&record.provenance.producer),
            inputs: record.provenance.inputs.iter().map(ref_to_file).collect(),
        },
        summary: summary_to_file(&record.summary),
    }
}

fn artefact_from_file(file: ArtefactFile) -> Result<ArtefactRecord, RunRecordError> {
    use crate::workflows::artefacts::ArtefactProvenance;
    Ok(ArtefactRecord {
        id: super::id::ArtefactId::parse(&file.id).ok_or(RunRecordError::Corrupt)?,
        kind: crate::workflows::definition::ArtefactKind::parse(&file.kind)
            .ok_or(RunRecordError::Corrupt)?,
        artefact_hash: crate::workflows::artefacts::ArtefactHash::parse(&file.artefact_hash)
            .ok_or(RunRecordError::Corrupt)?,
        object_hash: crate::workflows::artefacts::ObjectHash::parse(&file.object_hash)
            .ok_or(RunRecordError::Corrupt)?,
        payload_bytes: file.payload_bytes,
        created_at_ms: file.created_at_ms,
        provenance: ArtefactProvenance {
            run_id: RunId::parse(&file.provenance.run_id).ok_or(RunRecordError::Corrupt)?,
            producer: producer_from_file(file.provenance.producer)?,
            inputs: file
                .provenance
                .inputs
                .into_iter()
                .map(ref_from_file)
                .collect::<Result<Vec<_>, _>>()?,
        },
        summary: summary_from_file(file.summary)?,
    })
}

fn producer_to_file(producer: &crate::workflows::artefacts::ArtefactProducer) -> ProducerFile {
    use crate::workflows::artefacts::ArtefactProducer;
    match producer {
        ArtefactProducer::RunSourceCapture => ProducerFile::RunSourceCapture,
        ArtefactProducer::StepAttempt {
            attempt_id,
            step,
            output,
            disposition,
        } => ProducerFile::StepAttempt {
            attempt_id: attempt_id.as_hex(),
            step: step.as_str().to_owned(),
            output: output.as_ref().map(|item| item.as_str().to_owned()),
            disposition: disposition.as_str().to_owned(),
        },
        ArtefactProducer::HumanGate {
            gate_id,
            step,
            output,
        } => ProducerFile::HumanGate {
            gate_id: gate_id.as_hex(),
            step: step.as_str().to_owned(),
            output: output.as_str().to_owned(),
        },
    }
}

fn producer_from_file(
    file: ProducerFile,
) -> Result<crate::workflows::artefacts::ArtefactProducer, RunRecordError> {
    use crate::workflows::artefacts::{ArtefactProducer, ProductionDisposition};
    Ok(match file {
        ProducerFile::RunSourceCapture => ArtefactProducer::RunSourceCapture,
        ProducerFile::StepAttempt {
            attempt_id,
            step,
            output,
            disposition,
        } => ArtefactProducer::StepAttempt {
            attempt_id: AttemptId::parse(&attempt_id).ok_or(RunRecordError::Corrupt)?,
            step: StepKey::parse(&step).map_err(|_| RunRecordError::Corrupt)?,
            output: output
                .map(|value| OutputKey::parse(&value).map_err(|_| RunRecordError::Corrupt))
                .transpose()?,
            disposition: ProductionDisposition::parse(&disposition)
                .ok_or(RunRecordError::Corrupt)?,
        },
        ProducerFile::HumanGate {
            gate_id,
            step,
            output,
        } => ArtefactProducer::HumanGate {
            gate_id: GateId::parse(&gate_id).ok_or(RunRecordError::Corrupt)?,
            step: StepKey::parse(&step).map_err(|_| RunRecordError::Corrupt)?,
            output: OutputKey::parse(&output).map_err(|_| RunRecordError::Corrupt)?,
        },
    })
}

fn summary_to_file(summary: &crate::workflows::artefacts::ArtefactSummary) -> SummaryFile {
    use crate::workflows::artefacts::ArtefactSummary;
    match summary {
        ArtefactSummary::Plan { markdown_bytes } => SummaryFile::Plan {
            markdown_bytes: *markdown_bytes,
        },
        ArtefactSummary::Review { candidate, verdict } => SummaryFile::Review {
            candidate: candidate.as_str(),
            verdict: match verdict {
                crate::workflows::artefacts::ReviewVerdict::Approved => "approved".to_owned(),
                crate::workflows::artefacts::ReviewVerdict::RevisionRequired => {
                    "revision-required".to_owned()
                }
                crate::workflows::artefacts::ReviewVerdict::Blocked => "blocked".to_owned(),
            },
        },
        ArtefactSummary::Test { candidate, outcome } => SummaryFile::Test {
            candidate: candidate.as_str(),
            outcome: match outcome {
                crate::workflows::artefacts::TestOutcome::Passed => "passed".to_owned(),
                crate::workflows::artefacts::TestOutcome::Failed => "failed".to_owned(),
                crate::workflows::artefacts::TestOutcome::NotRun => "not-run".to_owned(),
            },
        },
        ArtefactSummary::Candidate {
            candidate,
            entries,
            bytes,
            disposition,
        } => SummaryFile::Candidate {
            candidate: candidate.as_str(),
            entries: *entries,
            bytes: *bytes,
            disposition: disposition.as_str().to_owned(),
        },
        ArtefactSummary::HumanDecision {
            candidate,
            diff_base,
            decision,
        } => SummaryFile::HumanDecision {
            candidate: candidate.as_str(),
            diff_base: diff_base.as_str(),
            decision: match decision {
                super::gates::HumanDecisionKind::Approved => "approved",
                super::gates::HumanDecisionKind::RevisionRequested => "revision-requested",
            }
            .to_owned(),
        },
    }
}

fn summary_from_file(
    file: SummaryFile,
) -> Result<crate::workflows::artefacts::ArtefactSummary, RunRecordError> {
    use crate::workflows::artefacts::{ArtefactSummary, ProductionDisposition};
    Ok(match file {
        SummaryFile::Plan { markdown_bytes } => ArtefactSummary::Plan { markdown_bytes },
        SummaryFile::Review { candidate, verdict } => ArtefactSummary::Review {
            candidate: crate::workflows::artefacts::CandidateHash::parse(&candidate)
                .ok_or(RunRecordError::Corrupt)?,
            verdict: match verdict.as_str() {
                "approved" => crate::workflows::artefacts::ReviewVerdict::Approved,
                "revision-required" => crate::workflows::artefacts::ReviewVerdict::RevisionRequired,
                "blocked" => crate::workflows::artefacts::ReviewVerdict::Blocked,
                _ => return Err(RunRecordError::Corrupt),
            },
        },
        SummaryFile::Test { candidate, outcome } => ArtefactSummary::Test {
            candidate: crate::workflows::artefacts::CandidateHash::parse(&candidate)
                .ok_or(RunRecordError::Corrupt)?,
            outcome: match outcome.as_str() {
                "passed" => crate::workflows::artefacts::TestOutcome::Passed,
                "failed" => crate::workflows::artefacts::TestOutcome::Failed,
                "not-run" => crate::workflows::artefacts::TestOutcome::NotRun,
                _ => return Err(RunRecordError::Corrupt),
            },
        },
        SummaryFile::Candidate {
            candidate,
            entries,
            bytes,
            disposition,
        } => ArtefactSummary::Candidate {
            candidate: crate::workflows::artefacts::CandidateHash::parse(&candidate)
                .ok_or(RunRecordError::Corrupt)?,
            entries,
            bytes,
            disposition: ProductionDisposition::parse(&disposition)
                .ok_or(RunRecordError::Corrupt)?,
        },
        SummaryFile::HumanDecision {
            candidate,
            diff_base,
            decision,
        } => ArtefactSummary::HumanDecision {
            candidate: crate::workflows::artefacts::CandidateHash::parse(&candidate)
                .ok_or(RunRecordError::Corrupt)?,
            diff_base: crate::workflows::artefacts::CandidateHash::parse(&diff_base)
                .ok_or(RunRecordError::Corrupt)?,
            decision: match decision.as_str() {
                "approved" => super::gates::HumanDecisionKind::Approved,
                "revision-requested" => super::gates::HumanDecisionKind::RevisionRequested,
                _ => return Err(RunRecordError::Corrupt),
            },
        },
    })
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn required_output_keys(action: &StepAction) -> Vec<String> {
    let outputs = match action {
        StepAction::Agent(action) => &action.required_outputs,
        StepAction::SystemCommand(action) => &action.required_outputs,
        StepAction::HumanGate(action) => std::slice::from_ref(&action.required_output),
    };
    outputs
        .iter()
        .map(|output| output.key.as_str().to_owned())
        .collect()
}

fn next_ordinal(attempts: &[AttemptRecord], step: &StepKey) -> u32 {
    attempts
        .iter()
        .filter(|attempt| attempt.step == *step)
        .count() as u32
        + 1
}

fn commit_transaction_to_file(transaction: &CommitTransaction) -> CommitTransactionFile {
    CommitTransactionFile {
        state: transaction.state.encode(),
        candidate: ref_to_file(&transaction.candidate),
        review: ref_to_file(&transaction.review),
        approval: transaction.approval.as_ref().map(ref_to_file),
        expected_reference: transaction.expected_reference.clone(),
        old_object: transaction.old_object.clone(),
        target_tree: transaction.target_tree.clone(),
        expected_commit: transaction.expected_commit.clone(),
        timestamp: transaction.timestamp.clone(),
    }
}

fn commit_transaction_from_file(
    file: CommitTransactionFile,
) -> Result<CommitTransaction, RunRecordError> {
    Ok(CommitTransaction {
        state: CommitTransactionState::parse(&file.state).ok_or(RunRecordError::Corrupt)?,
        candidate: ref_from_file(file.candidate)?,
        review: ref_from_file(file.review)?,
        approval: file.approval.map(ref_from_file).transpose()?,
        expected_reference: file.expected_reference,
        old_object: file.old_object,
        target_tree: file.target_tree,
        expected_commit: file.expected_commit,
        timestamp: file.timestamp,
    })
}

fn validate_attempt_result(
    attempt: &AttemptRecord,
    action: &StepDefinition,
) -> Result<(), RunRecordError> {
    let valid_finish = attempt
        .finished_at_ms
        .is_some_and(|finished| finished >= attempt.started_at_ms);
    match (&attempt.state, &attempt.finished_at_ms, &attempt.result) {
        (AttemptState::Active, None, None) => Ok(()),
        (AttemptState::Completed, Some(_), Some(AttemptResult::Completed { outputs }))
            if valid_finish
                && *outputs == required_output_keys(&action.action)
                && durable_outputs_match(attempt, action) =>
        {
            Ok(())
        }
        (AttemptState::Failed, Some(_), Some(AttemptResult::Failed { .. })) if valid_finish => {
            Ok(())
        }
        (AttemptState::Cancelled, Some(_), Some(AttemptResult::Cancelled)) if valid_finish => {
            Ok(())
        }
        (AttemptState::Interrupted, Some(_), Some(AttemptResult::Interrupted)) if valid_finish => {
            Ok(())
        }
        _ => Err(RunRecordError::Corrupt),
    }
}

fn durable_outputs_match(attempt: &AttemptRecord, step: &StepDefinition) -> bool {
    let fixes_or_commits = step
        .required_outputs()
        .iter()
        .any(|output| output.kind == crate::workflows::definition::OutputKind::ReviewReport)
        || matches!(
            &step.action,
            StepAction::SystemCommand(action)
                if action.command == crate::workflows::commands::SystemCommandId::CommitCandidate
        );
    if !fixes_or_commits {
        return true;
    }
    let durable: Vec<_> = step
        .required_outputs()
        .iter()
        .filter_map(|output| {
            output
                .kind
                .as_artefact_kind()
                .map(|kind| (&output.key, kind))
        })
        .collect();
    attempt.outputs.len() == durable.len()
        && durable.iter().all(|(key, kind)| {
            attempt
                .outputs
                .iter()
                .any(|output| output.key == **key && output.artefact.kind == *kind)
        })
}

fn validate_report_binding(
    attempt: &AttemptRecord,
    step: &StepDefinition,
    run: &WorkflowRun,
) -> Result<(), RunRecordError> {
    let StepAction::Agent(action) = &step.action else {
        return Ok(());
    };
    let reports: Vec<_> = attempt
        .outputs
        .iter()
        .filter(|output| {
            output.artefact.kind == crate::workflows::definition::ArtefactKind::ReviewReport
        })
        .collect();
    if reports.is_empty() {
        return Ok(());
    }
    let candidate_reference = match action.candidate_authority {
        crate::workflows::definition::CandidateAuthority::ReadOnly => attempt
            .inputs
            .iter()
            .find(|input| {
                input.artefact.kind == crate::workflows::definition::ArtefactKind::CandidateRevision
            })
            .map(|input| &input.artefact),
        crate::workflows::definition::CandidateAuthority::Edit => attempt
            .outputs
            .iter()
            .find(|output| {
                output.artefact.kind
                    == crate::workflows::definition::ArtefactKind::CandidateRevision
            })
            .map(|output| &output.artefact),
    }
    .ok_or(RunRecordError::Corrupt)?;
    let candidate_record = run
        .artefact(&candidate_reference.id)
        .ok_or(RunRecordError::Corrupt)?;
    let candidate = candidate_record
        .candidate_hash()
        .ok_or(RunRecordError::Corrupt)?;
    if candidate_record.provenance.run_id != run.id {
        return Err(RunRecordError::Corrupt);
    }
    if action.candidate_authority == crate::workflows::definition::CandidateAuthority::Edit
        && !required_output_from_attempt(candidate_record, attempt)
    {
        return Err(RunRecordError::Corrupt);
    }
    for report in reports {
        let record = run
            .artefact(&report.artefact.id)
            .ok_or(RunRecordError::Corrupt)?;
        let bound = record.candidate_hash().ok_or(RunRecordError::Corrupt)?;
        if bound != candidate
            || record.provenance.run_id != run.id
            || !required_output_from_attempt(record, attempt)
            || !record
                .provenance
                .inputs
                .iter()
                .any(|input| input == candidate_reference)
        {
            return Err(RunRecordError::Corrupt);
        }
    }
    Ok(())
}

fn required_output_from_attempt(
    record: &crate::workflows::artefacts::ArtefactRecord,
    attempt: &AttemptRecord,
) -> bool {
    matches!(
        &record.provenance.producer,
        crate::workflows::artefacts::ArtefactProducer::StepAttempt {
            attempt_id,
            step,
            disposition: crate::workflows::artefacts::ProductionDisposition::RequiredOutput,
            ..
        } if *attempt_id == attempt.id && *step == attempt.step
    )
}

fn validate_attempt_isolation(
    attempt: &AttemptRecord,
    step: &StepDefinition,
    run: &WorkflowRun,
) -> Result<(), RunRecordError> {
    if attempt.sandbox.kind != AttemptSandboxKind::IsolatedAttempt {
        return Err(RunRecordError::Corrupt);
    }
    let Some(binding) = run
        .environments
        .steps
        .iter()
        .find(|item| item.step == attempt.step)
    else {
        return Err(RunRecordError::Corrupt);
    };
    if attempt.sandbox.snapshot_digest != binding.snapshot_digest {
        return Err(RunRecordError::Corrupt);
    }
    let commit = matches!(
        &step.action,
        StepAction::SystemCommand(action)
            if action.command == crate::workflows::commands::SystemCommandId::CommitCandidate
    );
    if commit {
        if attempt.capabilities.git_admin != AccessMode::ReadWrite
            || attempt.capabilities.source_location != PrimarySourceLocation::UserProject
        {
            return Err(RunRecordError::Corrupt);
        }
        if let Some(transaction) = &attempt.commit_transaction
            && !valid_commit_transaction(attempt, transaction)
        {
            return Err(RunRecordError::Corrupt);
        }
        if let Some(result) = &attempt.commit_result {
            let Some(transaction) = &attempt.commit_transaction else {
                return Err(RunRecordError::Corrupt);
            };
            if transaction.verified_commit() != Some(result.commit.as_str())
                || !valid_git_object_id(&result.commit)
            {
                return Err(RunRecordError::Corrupt);
            }
        }
        if attempt.state == AttemptState::Completed
            && (attempt.commit_result.is_none() || attempt.commit_transaction.is_none())
        {
            return Err(RunRecordError::Corrupt);
        }
    } else if attempt.commit_transaction.is_some()
        || attempt.commit_result.is_some()
        || attempt.capabilities.git_admin != AccessMode::ReadOnly
        || attempt.capabilities.source_location != PrimarySourceLocation::AttemptWorkspace
    {
        return Err(RunRecordError::Corrupt);
    }
    match attempt.action_kind {
        ActionKind::SystemCommand => {
            if attempt.capabilities.network != NetworkCapability::None
                || attempt.capabilities.secret != SecretPresence::None
                || !attempt.capabilities.tools.is_empty()
            {
                return Err(RunRecordError::Corrupt);
            }
        }
        ActionKind::Agent => {}
        ActionKind::HumanGate => return Err(RunRecordError::Corrupt),
    }
    if !capabilities_match_step(&attempt.capabilities, step) {
        return Err(RunRecordError::Corrupt);
    }
    match (&attempt.state, &attempt.cleanup) {
        (AttemptState::Completed, AttemptCleanupRecord::Complete) => {}
        (AttemptState::Active, AttemptCleanupRecord::Pending) => {}
        (
            AttemptState::Interrupted,
            AttemptCleanupRecord::Pending
            | AttemptCleanupRecord::Complete
            | AttemptCleanupRecord::Orphaned { .. },
        ) => {}
        (
            AttemptState::Failed | AttemptState::Cancelled,
            AttemptCleanupRecord::Complete | AttemptCleanupRecord::Orphaned { .. },
        ) => {}
        _ => return Err(RunRecordError::Corrupt),
    }
    Ok(())
}

fn valid_commit_transaction(attempt: &AttemptRecord, transaction: &CommitTransaction) -> bool {
    if transaction.candidate.kind != crate::workflows::definition::ArtefactKind::CandidateRevision
        || transaction.review.kind != crate::workflows::definition::ArtefactKind::ReviewReport
        || transaction.approval.as_ref().is_some_and(|approval| {
            approval.kind != crate::workflows::definition::ArtefactKind::HumanDecision
                || !attempt
                    .inputs
                    .iter()
                    .any(|input| input.artefact == *approval)
        })
        || !attempt
            .inputs
            .iter()
            .any(|input| input.artefact == transaction.candidate)
        || !attempt
            .inputs
            .iter()
            .any(|input| input.artefact == transaction.review)
        || transaction.expected_reference.is_empty()
        || transaction.timestamp.split_once(' ').is_none()
        || transaction
            .old_object
            .as_deref()
            .is_some_and(|object| !valid_git_object_id(object))
        || transaction
            .target_tree
            .as_deref()
            .is_some_and(|object| !valid_git_object_id(object))
        || transaction
            .expected_commit
            .as_deref()
            .is_some_and(|object| !valid_git_object_id(object))
    {
        return false;
    }
    match &transaction.state {
        CommitTransactionState::Prepared => true,
        CommitTransactionState::WorktreeApplied => {
            transaction.target_tree.is_some() && transaction.expected_commit.is_some()
        }
        CommitTransactionState::ReferenceUpdated { commit }
        | CommitTransactionState::Verified { commit } => {
            transaction.target_tree.is_some()
                && transaction.expected_commit.as_deref() == Some(commit)
                && valid_git_object_id(commit)
        }
    }
}

fn valid_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn capabilities_match_step(capabilities: &AttemptCapabilities, step: &StepDefinition) -> bool {
    match &step.action {
        StepAction::Agent(action) => {
            let primary = capabilities
                .directories
                .iter()
                .filter(|directory| directory.role == DirectoryRole::PrimarySource);
            let primary: Vec<_> = primary.collect();
            let secondary: Vec<_> = capabilities
                .directories
                .iter()
                .filter(|directory| directory.role == DirectoryRole::SecondaryContext)
                .collect();
            capabilities.git_admin == AccessMode::ReadOnly
                && capabilities.source_location == PrimarySourceLocation::AttemptWorkspace
                && primary.len() == 1
                && primary[0].access == action.candidate_authority.access()
                && capabilities
                    .tools
                    .iter()
                    .all(|tool| action.authority.tools.contains(tool))
                && secondary.len() == action.authority.directories.len()
                && secondary.iter().all(|directory| {
                    valid_guest_path(&directory.guest_path)
                        && directory.access == AccessMode::ReadOnly
                        && action
                            .authority
                            .directories
                            .iter()
                            .any(|item| item.alias == directory.alias)
                })
        }
        StepAction::SystemCommand(action) => {
            let commit =
                action.command == crate::workflows::commands::SystemCommandId::CommitCandidate;
            capabilities.tools.is_empty()
                && capabilities.network == NetworkCapability::None
                && capabilities.secret == SecretPresence::None
                && capabilities.directories.iter().all(|directory| {
                    valid_guest_path(&directory.guest_path)
                        && if commit {
                            directory.access == AccessMode::ReadWrite
                        } else {
                            directory.access == AccessMode::ReadOnly
                        }
                })
                && if commit {
                    capabilities.git_admin == AccessMode::ReadWrite
                        && capabilities.source_location == PrimarySourceLocation::UserProject
                } else {
                    capabilities.git_admin == AccessMode::ReadOnly
                        && capabilities.source_location == PrimarySourceLocation::AttemptWorkspace
                }
        }
        StepAction::HumanGate(_) => false,
    }
}

fn valid_guest_path(path: &str) -> bool {
    (path == crate::agents::GUEST_PROJECT || path.starts_with("/access/"))
        && !path.contains('\\')
        && !path.contains(':')
        && !path.contains("..")
        && !path.contains("workflow-workspaces")
}

fn capabilities_to_file(capabilities: &AttemptCapabilities) -> AttemptCapabilitiesFile {
    AttemptCapabilitiesFile {
        tools: capabilities
            .tools
            .iter()
            .map(|tool| tool.as_str().to_owned())
            .collect(),
        directories: capabilities
            .directories
            .iter()
            .map(|directory| CapabilityDirectoryFile {
                alias: directory.alias.clone(),
                guest_path: directory.guest_path.clone(),
                access: directory.access.as_str().to_owned(),
                role: directory.role.as_str().to_owned(),
            })
            .collect(),
        git_admin: capabilities.git_admin.as_str().to_owned(),
        source_location: capabilities.source_location.as_str().to_owned(),
        network: capabilities.network.as_str().to_owned(),
        secret: capabilities.secret.as_str().to_owned(),
    }
}

fn capabilities_from_file(
    file: AttemptCapabilitiesFile,
) -> Result<AttemptCapabilities, RunRecordError> {
    let git_admin = AccessMode::parse(&file.git_admin).ok_or(RunRecordError::Corrupt)?;
    let source_location =
        PrimarySourceLocation::parse(&file.source_location).ok_or(RunRecordError::Corrupt)?;
    match (git_admin, source_location) {
        (AccessMode::ReadOnly, PrimarySourceLocation::AttemptWorkspace)
        | (AccessMode::ReadWrite, PrimarySourceLocation::UserProject) => {}
        _ => return Err(RunRecordError::Corrupt),
    }
    let mut tools = Vec::new();
    for name in file.tools {
        let tool = ToolId::parse(&name).ok_or(RunRecordError::Corrupt)?;
        if tools.contains(&tool) {
            return Err(RunRecordError::Corrupt);
        }
        tools.push(tool);
    }
    let mut directories = Vec::new();
    for directory in file.directories {
        if !valid_guest_path(&directory.guest_path) {
            return Err(RunRecordError::Corrupt);
        }
        directories.push(CapabilityDirectory {
            alias: directory.alias,
            guest_path: directory.guest_path,
            access: AccessMode::parse(&directory.access).ok_or(RunRecordError::Corrupt)?,
            role: DirectoryRole::parse(&directory.role).ok_or(RunRecordError::Corrupt)?,
        });
    }
    Ok(AttemptCapabilities {
        tools,
        directories,
        source_location,
        git_admin,
        network: NetworkCapability::parse(&file.network).ok_or(RunRecordError::Corrupt)?,
        secret: SecretPresence::parse(&file.secret).ok_or(RunRecordError::Corrupt)?,
    })
}

fn cleanup_to_file(cleanup: &AttemptCleanupRecord) -> AttemptCleanupFile {
    match cleanup {
        AttemptCleanupRecord::Pending => AttemptCleanupFile::Pending,
        AttemptCleanupRecord::Complete => AttemptCleanupFile::Complete,
        AttemptCleanupRecord::Orphaned {
            sandbox,
            workspace,
            journal,
        } => AttemptCleanupFile::Orphaned {
            sandbox: *sandbox,
            workspace: *workspace,
            journal: *journal,
        },
    }
}

fn cleanup_from_file(file: AttemptCleanupFile) -> Result<AttemptCleanupRecord, RunRecordError> {
    Ok(match file {
        AttemptCleanupFile::Pending => AttemptCleanupRecord::Pending,
        AttemptCleanupFile::Complete => AttemptCleanupRecord::Complete,
        AttemptCleanupFile::Orphaned {
            sandbox,
            workspace,
            journal,
        } => AttemptCleanupRecord::Orphaned {
            sandbox,
            workspace,
            journal,
        },
    })
}

impl AttemptSandboxKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::IsolatedAttempt => "isolated-attempt",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "isolated-attempt" => Some(Self::IsolatedAttempt),
            _ => None,
        }
    }
}

fn validate_transitions(
    created_at_ms: u64,
    definition: &WorkflowDefinition,
    attempts: &[AttemptRecord],
    gates: &[HumanGateRecord],
    transitions: &[TransitionRecord],
    state: &RunState,
) -> Result<(), RunRecordError> {
    let mut expected_state = RunState::Ready {
        step: definition.first_step().clone(),
    };
    let mut previous_time = created_at_ms;
    let mut started_attempts = Vec::new();
    for (index, transition) in transitions.iter().enumerate() {
        let sequence = u64::try_from(index + 1).map_err(|_| RunRecordError::Corrupt)?;
        if transition.sequence != sequence
            || transition.occurred_at_ms < previous_time
            || transition.from != expected_state
        {
            return Err(RunRecordError::Corrupt);
        }
        match transition.cause {
            TransitionCause::InitialSourceCaptureStarted => {
                if !matches!(
                    transition.from,
                    RunState::Ready { .. } | RunState::InitialisingSource
                ) || transition.to != RunState::InitialisingSource
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
            TransitionCause::InitialSourceCaptured => {
                if transition.from != RunState::InitialisingSource
                    || !matches!(transition.to, RunState::Ready { .. })
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
            TransitionCause::InitialSourceCaptureFailed => {
                if !matches!(
                    transition.from,
                    RunState::InitialisingSource | RunState::Ready { .. }
                ) || transition.to != RunState::Failed
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
            TransitionCause::SourceDriftDetected => {
                if transition.to != RunState::Failed {
                    return Err(RunRecordError::Corrupt);
                }
            }
            TransitionCause::AttemptStarted => {
                let (
                    RunState::Ready { step: from_step },
                    RunState::Active {
                        step: to_step,
                        attempt,
                    },
                ) = (&transition.from, &transition.to)
                else {
                    return Err(RunRecordError::Corrupt);
                };
                let record = attempts
                    .iter()
                    .find(|record| record.id == *attempt)
                    .ok_or(RunRecordError::Corrupt)?;
                if from_step != to_step
                    || record.step != *from_step
                    || record.started_at_ms != transition.occurred_at_ms
                    || started_attempts.contains(attempt)
                {
                    return Err(RunRecordError::Corrupt);
                }
                started_attempts.push(*attempt);
            }
            TransitionCause::AttemptCompleted => {
                let RunState::Active { step, attempt } = &transition.from else {
                    return Err(RunRecordError::Corrupt);
                };
                let record = attempts
                    .iter()
                    .find(|record| record.id == *attempt)
                    .ok_or(RunRecordError::Corrupt)?;
                let definition_step = definition.step(step).ok_or(RunRecordError::Corrupt)?;
                let expected_to = match &definition_step.on_success {
                    SuccessTransition::Next(next) => RunState::Ready { step: next.clone() },
                    SuccessTransition::CompleteRun => RunState::Completed,
                };
                if record.state != AttemptState::Completed
                    || record.finished_at_ms != Some(transition.occurred_at_ms)
                    || transition.to != expected_to
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
            TransitionCause::AttemptFailed => {
                let RunState::Active { attempt, .. } = &transition.from else {
                    return Err(RunRecordError::Corrupt);
                };
                let record = attempts
                    .iter()
                    .find(|record| record.id == *attempt)
                    .ok_or(RunRecordError::Corrupt)?;
                if record.state != AttemptState::Failed
                    || record.finished_at_ms != Some(transition.occurred_at_ms)
                    || transition.to != RunState::Failed
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
            TransitionCause::GateOpened => {
                let (
                    RunState::Ready { step },
                    RunState::AwaitingHuman {
                        step: to_step,
                        gate,
                    },
                ) = (&transition.from, &transition.to)
                else {
                    return Err(RunRecordError::Corrupt);
                };
                let record = gates
                    .iter()
                    .find(|item| item.id == *gate)
                    .ok_or(RunRecordError::Corrupt)?;
                if step != to_step
                    || record.step != *step
                    || record.revision.get() != transition.sequence
                    || record.opened_at_ms != transition.occurred_at_ms
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
            TransitionCause::GateApproved => {
                let RunState::AwaitingHuman { step, gate } = &transition.from else {
                    return Err(RunRecordError::Corrupt);
                };
                let record = gates
                    .iter()
                    .find(|item| item.id == *gate)
                    .ok_or(RunRecordError::Corrupt)?;
                let definition_step = definition.step(step).ok_or(RunRecordError::Corrupt)?;
                let expected_to = match &definition_step.on_success {
                    SuccessTransition::Next(next) => RunState::Ready { step: next.clone() },
                    SuccessTransition::CompleteRun => RunState::Completed,
                };
                if record.state != HumanGateState::Approved
                    || record.closed_at_ms != Some(transition.occurred_at_ms)
                    || transition.to != expected_to
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
            TransitionCause::GateRevisionRequested => {
                let (
                    RunState::AwaitingHuman { step, gate },
                    RunState::RevisionRequested {
                        step: to_step,
                        decision,
                    },
                ) = (&transition.from, &transition.to)
                else {
                    return Err(RunRecordError::Corrupt);
                };
                let record = gates
                    .iter()
                    .find(|item| item.id == *gate)
                    .ok_or(RunRecordError::Corrupt)?;
                if step != to_step
                    || record.state != HumanGateState::RevisionRequested
                    || record.decision.as_ref() != Some(decision)
                    || record.closed_at_ms != Some(transition.occurred_at_ms)
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
            TransitionCause::CancellationRequested => match &transition.from {
                RunState::InitialisingSource | RunState::Ready { .. }
                    if transition.to == RunState::Cancelled => {}
                RunState::AwaitingHuman { gate, .. } if transition.to == RunState::Cancelled => {
                    let record = gates
                        .iter()
                        .find(|item| item.id == *gate)
                        .ok_or(RunRecordError::Corrupt)?;
                    if record.state != HumanGateState::Cancelled || record.decision.is_some() {
                        return Err(RunRecordError::Corrupt);
                    }
                }
                RunState::Active { attempt, .. } if transition.to == RunState::Cancelled => {
                    let record = attempts
                        .iter()
                        .find(|record| record.id == *attempt)
                        .ok_or(RunRecordError::Corrupt)?;
                    if record.state != AttemptState::Cancelled
                        || record.finished_at_ms != Some(transition.occurred_at_ms)
                    {
                        return Err(RunRecordError::Corrupt);
                    }
                }
                _ => return Err(RunRecordError::Corrupt),
            },
            TransitionCause::ProcessRestarted => {
                if matches!(transition.from, RunState::InitialisingSource)
                    && transition.to == RunState::Interrupted
                {
                    previous_time = transition.occurred_at_ms;
                    expected_state = transition.to.clone();
                    continue;
                }
                if let RunState::AwaitingHuman { gate, .. } = &transition.from {
                    let record = gates
                        .iter()
                        .find(|item| item.id == *gate)
                        .ok_or(RunRecordError::Corrupt)?;
                    if record.state != HumanGateState::Interrupted
                        || transition.to != RunState::Interrupted
                    {
                        return Err(RunRecordError::Corrupt);
                    }
                    previous_time = transition.occurred_at_ms;
                    expected_state = transition.to.clone();
                    continue;
                }
                let RunState::Active { attempt, .. } = &transition.from else {
                    return Err(RunRecordError::Corrupt);
                };
                let record = attempts
                    .iter()
                    .find(|record| record.id == *attempt)
                    .ok_or(RunRecordError::Corrupt)?;
                if record.state != AttemptState::Interrupted
                    || record.finished_at_ms != Some(transition.occurred_at_ms)
                    || transition.to != RunState::Interrupted
                {
                    return Err(RunRecordError::Corrupt);
                }
            }
        }
        previous_time = transition.occurred_at_ms;
        expected_state = transition.to.clone();
    }
    if expected_state != *state
        || started_attempts
            != attempts
                .iter()
                .map(|attempt| attempt.id)
                .collect::<Vec<_>>()
    {
        return Err(RunRecordError::Corrupt);
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn next_ordinal_for(attempts: &[AttemptRecord], step: &StepKey) -> u32 {
    next_ordinal(attempts, step)
}

#[cfg(test)]
mod tests;
