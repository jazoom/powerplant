#[cfg(test)]
pub(crate) mod tests;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::agents::AgentId;
use crate::conversations::{ConversationId, DocumentId};
use crate::projects::ProjectId;

use super::artefacts::ArtefactReference;
use super::definition::PinnedWorkflowDefinition;
use super::id::{RunId, TaskLoopId};
use super::resolve::ResolvedEnvironmentSet;
use super::run::{PhaseModelSelection, RunState, TaskSelection, WorkflowRun};
use super::task_list::{MAXIMUM_TASK_LIST_BYTES, MAXIMUM_TASKS};

pub(crate) const LOOP_RECORD_VERSION: u32 = 1;
pub(crate) const MAXIMUM_LOOP_TASKS: usize = 32;
pub(crate) const MAXIMUM_LOOP_CHILDREN: usize = 32;
pub(crate) const MAXIMUM_LOOP_ATTEMPTS: usize = 256;
pub(crate) const MAXIMUM_LOOP_RECORD_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAXIMUM_LOOPS: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskLoop {
    pub(crate) id: TaskLoopId,
    pub(crate) created_at_ms: u64,
    pub(crate) conversation_id: ConversationId,
    pub(crate) project_id: ProjectId,
    pub(crate) agent_id: AgentId,
    pub(crate) launch_brief: String,
    pub(crate) pinned: PinnedWorkflowDefinition,
    pub(crate) phase_models: Vec<PhaseModelSelection>,
    pub(crate) environments: ResolvedEnvironmentSet,
    pub(crate) task_list: TaskListSnapshot,
    pub(crate) original_source: Option<ArtefactReference>,
    pub(crate) tasks: Vec<TaskLoopItem>,
    pub(crate) state: TaskLoopState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskListSnapshot {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: u32,
    pub(crate) content_hash: String,
    pub(crate) markdown: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskLoopItem {
    pub(crate) index: u32,
    pub(crate) markdown: String,
    pub(crate) child_id: Option<RunId>,
    pub(crate) outcome: TaskOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskOutcome {
    Pending,
    Reserved,
    Dispatched,
    CompletedCommit,
    CompletedUnchanged,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TaskLoopState {
    Ready,
    Active { task_index: u32, child: RunId },
    AwaitingChild { task_index: u32, child: RunId },
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LoopAdvance {
    Next,
    Complete,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskLoopError {
    Persist,
    Corrupt,
    Missing,
    Conflict,
    DuplicateDispatch,
    ChildLimit,
    AttemptLimit,
    TaskLimit,
    Empty,
}

impl TaskLoopError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Persist => "Power Plant could not store the task loop. Try again.",
            Self::Corrupt => "A task loop record is unreadable.",
            Self::Missing => "That task loop does not exist.",
            Self::Conflict => "Power Plant could not update that task loop.",
            Self::DuplicateDispatch => "That task already has a child run.",
            Self::ChildLimit => "The task loop cannot create another child.",
            Self::AttemptLimit => "The task loop reached its attempt bound.",
            Self::TaskLimit => "The task list has too many remaining tasks.",
            Self::Empty => "The task list has no remaining tasks.",
        }
    }
}

impl std::fmt::Display for TaskLoopError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for TaskLoopError {}

impl TaskLoopState {
    pub(crate) fn as_label(&self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Active { .. } => "Active",
            Self::AwaitingChild { .. } => "Awaiting decision",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Interrupted => "Interrupted",
            Self::Blocked => "Blocked",
        }
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted | Self::Blocked
        )
    }
}

impl TaskLoop {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        id: TaskLoopId,
        created_at_ms: u64,
        conversation_id: ConversationId,
        project_id: ProjectId,
        agent_id: AgentId,
        launch_brief: String,
        pinned: PinnedWorkflowDefinition,
        phase_models: Vec<PhaseModelSelection>,
        environments: ResolvedEnvironmentSet,
        task_list: TaskListSnapshot,
        tasks: Vec<TaskLoopItem>,
    ) -> Result<Self, TaskLoopError> {
        if pinned.definition.execution_mode() != super::definition::ExecutionMode::TaskList
            || !pinned.definition.supports_task_execution()
        {
            return Err(TaskLoopError::Conflict);
        }
        if tasks.is_empty() {
            return Err(TaskLoopError::Empty);
        }
        if tasks.len() > MAXIMUM_LOOP_TASKS || tasks.len() > MAXIMUM_TASKS {
            return Err(TaskLoopError::TaskLimit);
        }
        if task_list.markdown.len() > MAXIMUM_TASK_LIST_BYTES
            || task_list.revision == 0
            || crate::workflows::artefacts::ObjectHash::of(task_list.markdown.as_bytes()).as_str()
                != task_list.content_hash
        {
            return Err(TaskLoopError::Corrupt);
        }
        let record = Self {
            id,
            created_at_ms,
            conversation_id,
            project_id,
            agent_id,
            launch_brief,
            pinned,
            phase_models,
            environments,
            task_list,
            original_source: None,
            tasks,
            state: TaskLoopState::Ready,
        };
        record.validate()?;
        Ok(record)
    }

    pub(crate) fn current_child(&self) -> Option<RunId> {
        match self.state {
            TaskLoopState::Active { child, .. } | TaskLoopState::AwaitingChild { child, .. } => {
                Some(child)
            }
            _ => None,
        }
    }

    pub(crate) fn child_href(&self) -> String {
        self.current_child()
            .map(|child| format!("/runs/{}", child.as_hex()))
            .unwrap_or_default()
    }

    pub(crate) fn completed_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| {
                matches!(
                    task.outcome,
                    TaskOutcome::CompletedCommit | TaskOutcome::CompletedUnchanged
                )
            })
            .count()
    }

    pub(crate) fn progress_label(&self) -> String {
        format!(
            "Task {} of {}",
            self.completed_count()
                .saturating_add(usize::from(!self.state.is_terminal()))
                .min(self.tasks.len()),
            self.tasks.len()
        )
    }

    pub(crate) fn child_run(
        &self,
        child_id: RunId,
        created_at_ms: u64,
        index: u32,
        markdown: String,
    ) -> Result<WorkflowRun, TaskLoopError> {
        if self.current_child() != Some(child_id)
            || !self.tasks.iter().any(|task| {
                task.child_id == Some(child_id)
                    && matches!(
                        task.outcome,
                        TaskOutcome::Reserved | TaskOutcome::Dispatched
                    )
                    && task.index == index
                    && task.markdown == markdown
            })
        {
            return Err(TaskLoopError::Conflict);
        }
        let mut run = WorkflowRun::create_configured_for_conversation(
            child_id,
            created_at_ms,
            self.project_id,
            self.conversation_id,
            self.launch_brief.clone(),
            self.pinned.clone(),
            self.environments.clone(),
            self.phase_models.clone(),
        );
        run.agent_id = self.agent_id;
        run.set_parent_loop(self.id)
            .map_err(|_| TaskLoopError::Conflict)?;
        run.set_task_selection(TaskSelection {
            document_id: self.task_list.document_id,
            revision: self.task_list.revision,
            content_hash: self.task_list.content_hash.clone(),
            index,
            task_markdown: markdown,
            task_list: self.task_list.markdown.clone(),
        })
        .map_err(|_| TaskLoopError::Conflict)?;
        Ok(run)
    }

    fn validate(&self) -> Result<(), TaskLoopError> {
        if self.tasks.is_empty()
            || self.tasks.len() > MAXIMUM_LOOP_TASKS
            || super::input_context::validate_launch_brief(&self.launch_brief).is_err()
        {
            return Err(TaskLoopError::Corrupt);
        }
        if self.pinned.definition.execution_mode() != super::definition::ExecutionMode::TaskList
            || self.task_list.revision == 0
            || super::artefacts::ObjectHash::of(self.task_list.markdown.as_bytes()).as_str()
                != self.task_list.content_hash
        {
            return Err(TaskLoopError::Corrupt);
        }
        let parsed = super::task_list::parse(&self.task_list.markdown)
            .map_err(|_| TaskLoopError::Corrupt)?;
        let eligible: Vec<_> = parsed.eligible_tasks().collect();
        if eligible.len() != self.tasks.len() {
            return Err(TaskLoopError::Corrupt);
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut children = 0usize;
        for (task, authored) in self.tasks.iter().zip(eligible) {
            if task.index != authored.index || task.markdown != authored.markdown {
                return Err(TaskLoopError::Corrupt);
            }
            if let Some(child) = task.child_id {
                if !seen.insert(child) {
                    return Err(TaskLoopError::Corrupt);
                }
                children += 1;
            }
            if children > MAXIMUM_LOOP_CHILDREN {
                return Err(TaskLoopError::Corrupt);
            }
            match task.outcome {
                TaskOutcome::Pending if task.child_id.is_some() => {
                    return Err(TaskLoopError::Corrupt);
                }
                TaskOutcome::Reserved
                | TaskOutcome::Dispatched
                | TaskOutcome::CompletedCommit
                | TaskOutcome::CompletedUnchanged
                | TaskOutcome::Failed
                | TaskOutcome::Cancelled
                    if task.child_id.is_none() =>
                {
                    return Err(TaskLoopError::Corrupt);
                }
                _ => {}
            }
        }
        let active: Vec<_> = self
            .tasks
            .iter()
            .filter(|task| {
                matches!(
                    task.outcome,
                    TaskOutcome::Reserved | TaskOutcome::Dispatched
                )
            })
            .collect();
        match self.state {
            TaskLoopState::Ready if !active.is_empty() => return Err(TaskLoopError::Corrupt),
            TaskLoopState::Active { task_index, child }
            | TaskLoopState::AwaitingChild { task_index, child } => {
                if active.len() != 1
                    || active[0].index != task_index
                    || active[0].child_id != Some(child)
                {
                    return Err(TaskLoopError::Corrupt);
                }
            }
            TaskLoopState::Completed if self.completed_count() != self.tasks.len() => {
                return Err(TaskLoopError::Corrupt);
            }
            _ => {}
        }
        Ok(())
    }

    fn pending_index(&self) -> Option<usize> {
        self.tasks
            .iter()
            .position(|task| task.outcome == TaskOutcome::Pending && task.child_id.is_none())
    }

    fn item_mut(&mut self, child: RunId) -> Result<&mut TaskLoopItem, TaskLoopError> {
        self.tasks
            .iter_mut()
            .find(|task| task.child_id == Some(child))
            .ok_or(TaskLoopError::Conflict)
    }
}

pub(crate) struct TaskLoopStore {
    dir: Option<PathBuf>,
    inner: Mutex<BTreeMap<TaskLoopId, TaskLoop>>,
}

impl TaskLoopStore {
    pub(crate) fn open(dir: PathBuf) -> Result<Self, TaskLoopError> {
        crate::storage::ensure_private_dir(&dir).map_err(|_| TaskLoopError::Persist)?;
        let loops = load_dir(&dir)?;
        Ok(Self {
            dir: Some(dir),
            inner: Mutex::new(loops),
        })
    }

    pub(crate) fn create(&self, record: TaskLoop) -> Result<TaskLoop, TaskLoopError> {
        let mut loops = self.lock();
        if loops.len() >= MAXIMUM_LOOPS {
            return Err(TaskLoopError::ChildLimit);
        }
        if loops.contains_key(&record.id) {
            return Err(TaskLoopError::Conflict);
        }
        record.validate()?;
        persist(self.dir.as_deref(), &record)?;
        loops.insert(record.id, record.clone());
        Ok(record)
    }

    pub(crate) fn get(&self, id: &TaskLoopId) -> Option<TaskLoop> {
        self.lock().get(id).cloned()
    }

    pub(crate) fn for_conversation(&self, conversation: &ConversationId) -> Vec<TaskLoop> {
        let mut loops: Vec<_> = self
            .lock()
            .values()
            .filter(|record| record.conversation_id == *conversation)
            .cloned()
            .collect();
        loops.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then(right.id.cmp(&left.id))
        });
        loops
    }

    pub(crate) fn summaries(&self) -> Vec<LoopSummary> {
        let mut summaries: Vec<_> = self.lock().values().map(summary_of).collect();
        summaries.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then(right.id.cmp(&left.id))
        });
        summaries
    }

    pub(crate) fn mutate<F>(&self, id: &TaskLoopId, op: F) -> Result<TaskLoop, TaskLoopError>
    where
        F: FnOnce(&mut TaskLoop) -> Result<(), TaskLoopError>,
    {
        let mut loops = self.lock();
        let Some(current) = loops.get(id).cloned() else {
            return Err(TaskLoopError::Missing);
        };
        let mut next = current;
        op(&mut next)?;
        next.validate()?;
        persist(self.dir.as_deref(), &next)?;
        loops.insert(*id, next.clone());
        Ok(next)
    }

    pub(crate) fn reserve_next_child(
        &self,
        id: &TaskLoopId,
        aggregate_attempts: usize,
    ) -> Result<(TaskLoop, RunId, TaskLoopItem), TaskLoopError> {
        let result = self.mutate(id, |record| {
            if record.state != TaskLoopState::Ready {
                return Err(TaskLoopError::DuplicateDispatch);
            }
            if record.tasks.iter().filter(|task| task.child_id.is_some()).count()
                >= MAXIMUM_LOOP_CHILDREN
            {
                return Err(TaskLoopError::ChildLimit);
            }
            if aggregate_attempts >= MAXIMUM_LOOP_ATTEMPTS {
                return Err(TaskLoopError::AttemptLimit);
            }
            let index = record.pending_index().ok_or(TaskLoopError::Empty)?;
            if record.tasks[index].child_id.is_some() {
                return Err(TaskLoopError::DuplicateDispatch);
            }
            let child = RunId::generate().map_err(|_| TaskLoopError::Persist)?;
            record.tasks[index].child_id = Some(child);
            record.tasks[index].outcome = TaskOutcome::Reserved;
            record.state = TaskLoopState::Active {
                task_index: record.tasks[index].index,
                child,
            };
            Ok(())
        })
        .and_then(|record| {
            let task = record
                .tasks
                .iter()
                .find(|task| {
                    matches!(record.state, TaskLoopState::Active { child, .. } if task.child_id == Some(child))
                })
                .cloned()
                .ok_or(TaskLoopError::Conflict)?;
            let child = task.child_id.ok_or(TaskLoopError::Conflict)?;
            Ok((record, child, task))
        });
        if matches!(
            result,
            Err(TaskLoopError::ChildLimit | TaskLoopError::AttemptLimit)
        ) {
            self.mutate(id, |record| {
                record.state = TaskLoopState::Blocked;
                Ok(())
            })?;
        }
        result
    }

    pub(crate) fn mark_dispatched(
        &self,
        id: &TaskLoopId,
        child: RunId,
    ) -> Result<TaskLoop, TaskLoopError> {
        self.mutate(id, |record| {
            if record.current_child() != Some(child) {
                return Err(TaskLoopError::Conflict);
            }
            let task = record.item_mut(child)?;
            if task.outcome != TaskOutcome::Reserved {
                return Err(TaskLoopError::DuplicateDispatch);
            }
            task.outcome = TaskOutcome::Dispatched;
            Ok(())
        })
    }

    pub(crate) fn mark_awaiting(
        &self,
        id: &TaskLoopId,
        child: RunId,
    ) -> Result<TaskLoop, TaskLoopError> {
        self.mutate(id, |record| {
            let index = record
                .tasks
                .iter()
                .find(|task| task.child_id == Some(child))
                .map(|task| task.index)
                .ok_or(TaskLoopError::Conflict)?;
            if !matches!(
                record.state,
                TaskLoopState::Active { child: current, .. } if current == child
            ) && !matches!(
                record.state,
                TaskLoopState::AwaitingChild { child: current, .. } if current == child
            ) {
                return Err(TaskLoopError::Conflict);
            }
            record.state = TaskLoopState::AwaitingChild {
                task_index: index,
                child,
            };
            Ok(())
        })
    }

    pub(crate) fn mark_active(
        &self,
        id: &TaskLoopId,
        child: RunId,
    ) -> Result<TaskLoop, TaskLoopError> {
        self.mutate(id, |record| {
            if record.current_child() != Some(child) {
                return Err(TaskLoopError::Conflict);
            }
            let index = record
                .tasks
                .iter()
                .find(|task| task.child_id == Some(child))
                .map(|task| task.index)
                .ok_or(TaskLoopError::Conflict)?;
            record.state = TaskLoopState::Active {
                task_index: index,
                child,
            };
            Ok(())
        })
    }

    pub(crate) fn record_original_source(
        &self,
        id: &TaskLoopId,
        source: ArtefactReference,
    ) -> Result<TaskLoop, TaskLoopError> {
        self.mutate(id, |record| {
            if record.original_source.is_none() {
                record.original_source = Some(source);
            }
            Ok(())
        })
    }

    pub(crate) fn complete_child(
        &self,
        id: &TaskLoopId,
        child: &WorkflowRun,
    ) -> Result<(TaskLoop, LoopAdvance), TaskLoopError> {
        let mut advance = LoopAdvance::Stopped;
        let record = self.mutate(id, |record| {
            if child.parent_loop != Some(record.id) {
                return Err(TaskLoopError::Conflict);
            }
            if record.current_child() != Some(child.id) {
                return Err(TaskLoopError::DuplicateDispatch);
            }
            if child.conversation_id != Some(record.conversation_id)
                || child.project_id != record.project_id
                || child.pinned != record.pinned
                || child.phase_models != record.phase_models
            {
                return Err(TaskLoopError::Conflict);
            }
            let selection = child
                .task_selection
                .as_ref()
                .ok_or(TaskLoopError::Conflict)?;
            if selection.document_id != record.task_list.document_id
                || selection.revision != record.task_list.revision
                || selection.content_hash != record.task_list.content_hash
                || selection.task_list != record.task_list.markdown
            {
                return Err(TaskLoopError::Conflict);
            }
            let outcome = child_outcome(child).ok_or(TaskLoopError::Conflict)?;
            let task = record.item_mut(child.id)?;
            if task.outcome != TaskOutcome::Dispatched
                || selection.index != task.index
                || selection.task_markdown != task.markdown
            {
                return Err(TaskLoopError::Conflict);
            }
            if matches!(
                task.outcome,
                TaskOutcome::CompletedCommit | TaskOutcome::CompletedUnchanged
            ) {
                return Err(TaskLoopError::DuplicateDispatch);
            }
            task.outcome = outcome;
            if matches!(
                outcome,
                TaskOutcome::CompletedCommit | TaskOutcome::CompletedUnchanged
            ) {
                if record.pending_index().is_some() {
                    record.state = TaskLoopState::Ready;
                    advance = LoopAdvance::Next;
                } else {
                    record.state = TaskLoopState::Completed;
                    advance = LoopAdvance::Complete;
                }
            } else {
                record.state = match outcome {
                    TaskOutcome::Cancelled => TaskLoopState::Cancelled,
                    _ => TaskLoopState::Failed,
                };
                advance = LoopAdvance::Stopped;
            }
            Ok(())
        })?;
        Ok((record, advance))
    }

    pub(crate) fn fail(&self, id: &TaskLoopId) -> Result<TaskLoop, TaskLoopError> {
        self.mutate(id, |record| {
            if !record.state.is_terminal() {
                if let Some(child) = record.current_child() {
                    record.item_mut(child)?.outcome = TaskOutcome::Failed;
                }
                record.state = TaskLoopState::Failed;
            }
            Ok(())
        })
    }

    pub(crate) fn cancel(&self, id: &TaskLoopId) -> Result<TaskLoop, TaskLoopError> {
        self.mutate(id, |record| {
            if !record.state.is_terminal() {
                if let Some(child) = record.current_child() {
                    record.item_mut(child)?.outcome = TaskOutcome::Cancelled;
                }
                record.state = TaskLoopState::Cancelled;
            }
            Ok(())
        })
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<TaskLoopId, TaskLoop>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LoopSummary {
    pub(crate) id: TaskLoopId,
    pub(crate) project_id: ProjectId,
    pub(crate) name: String,
    pub(crate) state: String,
    pub(crate) created_at_ms: u64,
    pub(crate) current_step: String,
}

fn summary_of(record: &TaskLoop) -> LoopSummary {
    LoopSummary {
        id: record.id,
        project_id: record.project_id,
        name: record.pinned.definition.name().to_owned(),
        state: record.state.as_label().to_owned(),
        created_at_ms: record.created_at_ms,
        current_step: record.progress_label(),
    }
}

fn child_outcome(run: &WorkflowRun) -> Option<TaskOutcome> {
    match &run.state {
        RunState::Completed if run.completed_without_changes() => {
            Some(TaskOutcome::CompletedUnchanged)
        }
        RunState::Completed
            if run
                .attempts
                .iter()
                .any(|attempt| {
                    attempt.commit_result.is_some()
                        && attempt.state == super::run::AttemptState::Completed
                        && attempt.cleanup == super::run::AttemptCleanupRecord::Complete
                        && run.pinned.definition.step(&attempt.step).is_some_and(|step| {
                            matches!(&step.action, super::definition::StepAction::SystemCommand(action)
                                if action.command == super::definition::SystemCommandId::CommitCandidate)
                        })
                }) =>
        {
            Some(TaskOutcome::CompletedCommit)
        }
        RunState::Cancelled => Some(TaskOutcome::Cancelled),
        RunState::Failed | RunState::Escalated { .. } | RunState::Interrupted => {
            Some(TaskOutcome::Failed)
        }
        _ => None,
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct LoopFile {
    record_version: u32,
    id: String,
    created_at_ms: u64,
    conversation_id: String,
    project_id: String,
    agent_id: String,
    launch_brief: String,
    workflow_id: Option<String>,
    version: String,
    definition: super::definition::DefinitionFile,
    phase_models: Vec<PhaseModelFile>,
    environments: crate::workflows::run::ResolvedEnvironmentSetFile,
    task_list: TaskListFile,
    original_source: Option<ArtefactRefFile>,
    tasks: Vec<TaskItemFile>,
    state: LoopStateFile,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct TaskListFile {
    document_id: String,
    revision: u32,
    content_hash: String,
    markdown: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct TaskItemFile {
    index: u32,
    markdown: String,
    child_id: Option<String>,
    outcome: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum LoopStateFile {
    Ready,
    Active { task_index: u32, child: String },
    AwaitingChild { task_index: u32, child: String },
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    Blocked,
}

#[derive(Deserialize, Serialize)]
struct ArtefactRefFile {
    id: String,
    kind: String,
    artefact_hash: String,
}

#[derive(Deserialize, Serialize)]
struct PhaseModelFile {
    step: String,
    provider: String,
    model: String,
    thinking: Option<String>,
    instructions: String,
    preset: Option<PinnedPresetFile>,
}

#[derive(Deserialize, Serialize)]
struct PinnedPresetFile {
    id: String,
    revision: u32,
    name: String,
}

impl TaskLoop {
    fn to_file(&self) -> Result<LoopFile, TaskLoopError> {
        Ok(LoopFile {
            record_version: LOOP_RECORD_VERSION,
            id: self.id.as_hex(),
            created_at_ms: self.created_at_ms,
            conversation_id: self.conversation_id.as_hex(),
            project_id: self.project_id.as_hex(),
            agent_id: self.agent_id.as_hex(),
            launch_brief: self.launch_brief.clone(),
            workflow_id: self.pinned.workflow_id.map(|id| id.as_hex()),
            version: self.pinned.version.as_hex(),
            definition: self.pinned.definition.to_file(),
            phase_models: self.phase_models.iter().map(phase_model_to_file).collect(),
            environments: super::run::environment_set_to_file(&self.environments),
            task_list: TaskListFile {
                document_id: self.task_list.document_id.as_hex(),
                revision: self.task_list.revision,
                content_hash: self.task_list.content_hash.clone(),
                markdown: self.task_list.markdown.clone(),
            },
            original_source: self.original_source.as_ref().map(ref_to_file),
            tasks: self
                .tasks
                .iter()
                .map(|task| TaskItemFile {
                    index: task.index,
                    markdown: task.markdown.clone(),
                    child_id: task.child_id.map(|id| id.as_hex()),
                    outcome: outcome_as_str(task.outcome).to_owned(),
                })
                .collect(),
            state: state_to_file(&self.state),
        })
    }

    fn from_file(file: LoopFile) -> Result<Self, TaskLoopError> {
        if file.record_version != LOOP_RECORD_VERSION {
            return Err(TaskLoopError::Corrupt);
        }
        let definition = super::definition::WorkflowDefinition::from_file(file.definition)
            .map_err(|_| TaskLoopError::Corrupt)?;
        let version = definition.version();
        if version.as_hex() != file.version {
            return Err(TaskLoopError::Corrupt);
        }
        let record = Self {
            id: TaskLoopId::parse(&file.id).ok_or(TaskLoopError::Corrupt)?,
            created_at_ms: file.created_at_ms,
            conversation_id: ConversationId::parse(&file.conversation_id)
                .ok_or(TaskLoopError::Corrupt)?,
            project_id: ProjectId::parse(&file.project_id).ok_or(TaskLoopError::Corrupt)?,
            agent_id: AgentId::parse(&file.agent_id).ok_or(TaskLoopError::Corrupt)?,
            launch_brief: file.launch_brief,
            pinned: PinnedWorkflowDefinition {
                workflow_id: match file.workflow_id {
                    Some(value) => {
                        Some(super::WorkflowId::parse(&value).ok_or(TaskLoopError::Corrupt)?)
                    }
                    None => None,
                },
                version,
                definition,
            },
            phase_models: file
                .phase_models
                .into_iter()
                .map(phase_model_from_file)
                .collect::<Result<Vec<_>, _>>()?,
            environments: super::run::environment_set_from_file(file.environments)
                .map_err(|_| TaskLoopError::Corrupt)?,
            task_list: TaskListSnapshot {
                document_id: DocumentId::parse(&file.task_list.document_id)
                    .ok_or(TaskLoopError::Corrupt)?,
                revision: file.task_list.revision,
                content_hash: file.task_list.content_hash,
                markdown: file.task_list.markdown,
            },
            original_source: file.original_source.map(ref_from_file).transpose()?,
            tasks: file
                .tasks
                .into_iter()
                .map(task_from_file)
                .collect::<Result<Vec<_>, _>>()?,
            state: state_from_file(file.state)?,
        };
        record.validate()?;
        Ok(record)
    }
}

fn outcome_as_str(outcome: TaskOutcome) -> &'static str {
    match outcome {
        TaskOutcome::Pending => "pending",
        TaskOutcome::Reserved => "reserved",
        TaskOutcome::Dispatched => "dispatched",
        TaskOutcome::CompletedCommit => "completed-commit",
        TaskOutcome::CompletedUnchanged => "completed-unchanged",
        TaskOutcome::Failed => "failed",
        TaskOutcome::Cancelled => "cancelled",
    }
}

fn outcome_from_str(value: &str) -> Option<TaskOutcome> {
    match value {
        "pending" => Some(TaskOutcome::Pending),
        "reserved" => Some(TaskOutcome::Reserved),
        "dispatched" => Some(TaskOutcome::Dispatched),
        "completed-commit" => Some(TaskOutcome::CompletedCommit),
        "completed-unchanged" => Some(TaskOutcome::CompletedUnchanged),
        "failed" => Some(TaskOutcome::Failed),
        "cancelled" => Some(TaskOutcome::Cancelled),
        _ => None,
    }
}

fn task_from_file(file: TaskItemFile) -> Result<TaskLoopItem, TaskLoopError> {
    Ok(TaskLoopItem {
        index: file.index,
        markdown: file.markdown,
        child_id: match file.child_id {
            Some(value) => Some(RunId::parse(&value).ok_or(TaskLoopError::Corrupt)?),
            None => None,
        },
        outcome: outcome_from_str(&file.outcome).ok_or(TaskLoopError::Corrupt)?,
    })
}

fn state_to_file(state: &TaskLoopState) -> LoopStateFile {
    match state {
        TaskLoopState::Ready => LoopStateFile::Ready,
        TaskLoopState::Active { task_index, child } => LoopStateFile::Active {
            task_index: *task_index,
            child: child.as_hex(),
        },
        TaskLoopState::AwaitingChild { task_index, child } => LoopStateFile::AwaitingChild {
            task_index: *task_index,
            child: child.as_hex(),
        },
        TaskLoopState::Completed => LoopStateFile::Completed,
        TaskLoopState::Failed => LoopStateFile::Failed,
        TaskLoopState::Cancelled => LoopStateFile::Cancelled,
        TaskLoopState::Interrupted => LoopStateFile::Interrupted,
        TaskLoopState::Blocked => LoopStateFile::Blocked,
    }
}

fn state_from_file(file: LoopStateFile) -> Result<TaskLoopState, TaskLoopError> {
    Ok(match file {
        LoopStateFile::Ready => TaskLoopState::Ready,
        LoopStateFile::Active { task_index, child } => TaskLoopState::Active {
            task_index,
            child: RunId::parse(&child).ok_or(TaskLoopError::Corrupt)?,
        },
        LoopStateFile::AwaitingChild { task_index, child } => TaskLoopState::AwaitingChild {
            task_index,
            child: RunId::parse(&child).ok_or(TaskLoopError::Corrupt)?,
        },
        LoopStateFile::Completed => TaskLoopState::Completed,
        LoopStateFile::Failed => TaskLoopState::Failed,
        LoopStateFile::Cancelled => TaskLoopState::Cancelled,
        LoopStateFile::Interrupted => TaskLoopState::Interrupted,
        LoopStateFile::Blocked => TaskLoopState::Blocked,
    })
}

fn ref_to_file(reference: &ArtefactReference) -> ArtefactRefFile {
    ArtefactRefFile {
        id: reference.id.as_hex(),
        kind: reference.kind.as_str().to_owned(),
        artefact_hash: reference.artefact_hash.as_str().to_owned(),
    }
}

fn ref_from_file(file: ArtefactRefFile) -> Result<ArtefactReference, TaskLoopError> {
    Ok(ArtefactReference {
        id: super::ArtefactId::parse(&file.id).ok_or(TaskLoopError::Corrupt)?,
        kind: super::definition::ArtefactKind::parse(&file.kind).ok_or(TaskLoopError::Corrupt)?,
        artefact_hash: super::artefacts::ArtefactHash::parse(&file.artefact_hash)
            .ok_or(TaskLoopError::Corrupt)?,
    })
}

fn phase_model_to_file(selection: &PhaseModelSelection) -> PhaseModelFile {
    PhaseModelFile {
        step: selection.step.as_str().to_owned(),
        provider: selection.selection.provider.as_str().to_owned(),
        model: selection.selection.model.clone(),
        thinking: selection
            .selection
            .thinking
            .as_ref()
            .map(|effort| effort.as_str().to_owned()),
        instructions: selection.instructions.clone(),
        preset: selection.preset.as_ref().map(|preset| PinnedPresetFile {
            id: preset.id.as_hex(),
            revision: preset.revision,
            name: preset.name.clone(),
        }),
    }
}

fn phase_model_from_file(file: PhaseModelFile) -> Result<PhaseModelSelection, TaskLoopError> {
    use crate::providers::{ModelSelection, ProviderKind, ThinkingEffort};
    let thinking = match file.thinking.as_deref() {
        Some(value) => Some(ThinkingEffort::new(value.to_owned()).ok_or(TaskLoopError::Corrupt)?),
        None => None,
    };
    Ok(PhaseModelSelection {
        step: super::definition::StepKey::parse(&file.step).map_err(|_| TaskLoopError::Corrupt)?,
        selection: ModelSelection::new(
            ProviderKind::parse(&file.provider).ok_or(TaskLoopError::Corrupt)?,
            file.model,
            thinking,
        )
        .ok_or(TaskLoopError::Corrupt)?,
        instructions: file.instructions,
        preset: match file.preset {
            Some(preset) => Some(super::run::PinnedPreset {
                id: AgentId::parse(&preset.id).ok_or(TaskLoopError::Corrupt)?,
                revision: preset.revision,
                name: preset.name,
            }),
            None => None,
        },
    })
}

fn load_dir(dir: &Path) -> Result<BTreeMap<TaskLoopId, TaskLoop>, TaskLoopError> {
    let mut loops = BTreeMap::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(loops),
        Err(_) => return Err(TaskLoopError::Persist),
    };
    for entry in entries {
        let entry = entry.map_err(|_| TaskLoopError::Persist)?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = crate::storage::read_private_bounded(&path, MAXIMUM_LOOP_RECORD_BYTES)
            .map_err(|_| TaskLoopError::Corrupt)?;
        let file: LoopFile = serde_json::from_slice(&bytes).map_err(|_| TaskLoopError::Corrupt)?;
        let record = TaskLoop::from_file(file)?;
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or(TaskLoopError::Corrupt)?;
        if stem != record.id.as_hex() {
            return Err(TaskLoopError::Corrupt);
        }
        if loops.len() >= MAXIMUM_LOOPS || loops.insert(record.id, record).is_some() {
            return Err(TaskLoopError::Corrupt);
        }
    }
    Ok(loops)
}

fn persist(dir: Option<&Path>, record: &TaskLoop) -> Result<(), TaskLoopError> {
    let Some(dir) = dir else {
        return Ok(());
    };
    crate::storage::ensure_private_dir(dir).map_err(|_| TaskLoopError::Persist)?;
    let path = dir.join(format!("{}.json", record.id.as_hex()));
    let bytes =
        serde_json::to_vec_pretty(&record.to_file()?).map_err(|_| TaskLoopError::Persist)?;
    if bytes.len() > MAXIMUM_LOOP_RECORD_BYTES {
        return Err(TaskLoopError::Persist);
    }
    crate::storage::write_private(&path, &bytes).map_err(|_| TaskLoopError::Persist)
}
