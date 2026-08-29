use serde::{Deserialize, Serialize};

use super::definition::{
    DefinitionFile, DefinitionVersion, PinnedWorkflowDefinition, StepAction, StepDefinition,
    StepKey, SuccessTransition, WorkflowDefinition,
};
use super::id::{AttemptId, RunId, WorkflowId};

pub(crate) const RUN_RECORD_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkflowRun {
    pub(crate) id: RunId,
    pub(crate) created_at_ms: u64,
    pub(crate) pinned: PinnedWorkflowDefinition,
    pub(crate) state: RunState,
    pub(crate) attempts: Vec<AttemptRecord>,
    pub(crate) transitions: Vec<TransitionRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RunState {
    Ready { step: StepKey },
    Active { step: StepKey, attempt: AttemptId },
    Completed,
    Failed,
    Cancelled,
    Interrupted,
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
    AttemptStarted,
    AttemptCompleted,
    AttemptFailed,
    CancellationRequested,
    ProcessRestarted,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionKind {
    Agent,
    SystemCommand,
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
    workflow_id: Option<String>,
    version: String,
    definition: DefinitionFile,
    state: RunStateFile,
    attempts: Vec<AttemptFile>,
    transitions: Vec<TransitionFile>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum RunStateFile {
    Ready { step: String },
    Active { step: String, attempt: String },
    Completed,
    Failed,
    Cancelled,
    Interrupted,
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

impl WorkflowRun {
    pub(crate) fn create(id: RunId, created_at_ms: u64, pinned: PinnedWorkflowDefinition) -> Self {
        let step = pinned.definition.first_step().clone();
        Self {
            id,
            created_at_ms,
            pinned,
            state: RunState::Ready { step },
            attempts: Vec::new(),
            transitions: Vec::new(),
        }
    }

    pub(crate) fn start_attempt(
        &mut self,
        attempt_id: AttemptId,
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
        });
        let from = self.state.clone();
        let to = RunState::Active {
            step,
            attempt: attempt_id,
        };
        self.push_transition(at_ms, TransitionCause::AttemptStarted, from, to);
        Ok(())
    }

    pub(crate) fn complete_attempt(
        &mut self,
        attempt_id: AttemptId,
        outputs: Vec<String>,
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
        if outputs.len() != expected_outputs.len()
            || expected_outputs
                .iter()
                .any(|expected| !outputs.contains(expected))
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
            RunState::Completed
            | RunState::Failed
            | RunState::Cancelled
            | RunState::Interrupted => Err(TransitionError::Invalid),
        }
    }

    pub(crate) fn interrupt(&mut self, at_ms: u64) -> Result<(), TransitionError> {
        if !self.accepts_time(at_ms) {
            return Err(TransitionError::Invalid);
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
        matches!(self.state, RunState::Active { .. })
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            RunState::Completed | RunState::Failed | RunState::Cancelled | RunState::Interrupted
        )
    }

    pub(crate) fn current_step_name(&self) -> Option<&str> {
        let key = match &self.state {
            RunState::Ready { step } | RunState::Active { step, .. } => step,
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
            workflow_id: self.pinned.workflow_id.map(|id| id.as_hex()),
            version: self.pinned.version.as_hex(),
            definition: self.pinned.definition.to_file(),
            state: RunStateFile::from_state(&self.state),
            attempts: self.attempts.iter().map(AttemptRecord::to_file).collect(),
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
        let transitions = file
            .transitions
            .into_iter()
            .map(TransitionRecord::from_file)
            .collect::<Result<Vec<_>, _>>()?;
        let run = Self {
            id,
            created_at_ms: file.created_at_ms,
            pinned: PinnedWorkflowDefinition {
                workflow_id,
                version,
                definition,
            },
            state,
            attempts,
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
        }
        validate_transitions(
            self.created_at_ms,
            &self.pinned.definition,
            &self.attempts,
            &self.transitions,
            &self.state,
        )
    }
}

impl RunState {
    pub(crate) fn as_label(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "Ready",
            Self::Active { .. } => "Active",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Interrupted => "Interrupted",
        }
    }

    fn from_file(file: RunStateFile) -> Result<Self, RunRecordError> {
        Ok(match file {
            RunStateFile::Ready { step } => Self::Ready {
                step: StepKey::parse(&step).map_err(|_| RunRecordError::Corrupt)?,
            },
            RunStateFile::Active { step, attempt } => Self::Active {
                step: StepKey::parse(&step).map_err(|_| RunRecordError::Corrupt)?,
                attempt: AttemptId::parse(&attempt).ok_or(RunRecordError::Corrupt)?,
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
            RunState::Ready { step } => Self::Ready {
                step: step.as_str().to_owned(),
            },
            RunState::Active { step, attempt } => Self::Active {
                step: step.as_str().to_owned(),
                attempt: attempt.as_hex(),
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
        }
    }
}

impl ActionKind {
    fn from_action(action: &StepAction) -> Self {
        match action {
            StepAction::Agent(_) => Self::Agent,
            StepAction::SystemCommand(_) => Self::SystemCommand,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "agent" => Some(Self::Agent),
            "system-command" => Some(Self::SystemCommand),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::SystemCommand => "system-command",
        }
    }

    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::SystemCommand => "System command",
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
            "attempt-started" => Some(Self::AttemptStarted),
            "attempt-completed" => Some(Self::AttemptCompleted),
            "attempt-failed" => Some(Self::AttemptFailed),
            "cancellation-requested" => Some(Self::CancellationRequested),
            "process-restarted" => Some(Self::ProcessRestarted),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::AttemptStarted => "attempt-started",
            Self::AttemptCompleted => "attempt-completed",
            Self::AttemptFailed => "attempt-failed",
            Self::CancellationRequested => "cancellation-requested",
            Self::ProcessRestarted => "process-restarted",
        }
    }
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
            if valid_finish && *outputs == required_output_keys(&action.action) =>
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

fn validate_transitions(
    created_at_ms: u64,
    definition: &WorkflowDefinition,
    attempts: &[AttemptRecord],
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
            TransitionCause::CancellationRequested => match &transition.from {
                RunState::Ready { .. } if transition.to == RunState::Cancelled => {}
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
