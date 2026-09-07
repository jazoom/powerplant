#[cfg(test)]
pub(super) mod tests;

pub(crate) mod apply;
pub(crate) mod artefacts;
pub(crate) mod capabilities;
mod catalogue;
pub(crate) mod commands;
mod commit;
pub(crate) mod definition;
pub(crate) mod evidence;
mod execution;
mod executor;
pub(crate) mod gates;
mod id;
pub(crate) mod input_context;
mod quick;
mod resolve;
pub(crate) mod run;

pub(crate) mod seeds;
mod store;
pub(crate) mod summary;
pub(crate) mod task_list;
pub(crate) mod task_loop;
pub(crate) mod workspace;

pub(crate) use apply::ApplyJournals;
pub(crate) use artefacts::WorkflowArtefactRepository;
pub(crate) use catalogue::{
    CatalogueError, ResolveWorkflowError, WorkflowCatalogue, WorkflowRecord, WorkflowSelection,
    definition_fits_agent,
};
pub(crate) use commit::CommitJournals;
pub(crate) use evidence::{AttemptEvidenceContext, WorkflowEvidenceStore};
pub(crate) use execution::{ExecutionGuard, WorkflowExecution};
pub(crate) use executor::{
    PausedWorkflow, WorkflowContinuationRegistry, WorkflowJob, execute_run,
    interrupt_provider_continuations, interrupt_session_continuations, reconstruct_loop_job,
    recover_apply_transactions, recover_commit_transactions, recover_task_loops,
    settle_cancelled_job, settle_terminal_job, validate_phase_selection,
};
pub(crate) use id::{ArtefactId, AttemptId, GateId, RunId, TaskLoopId, WorkflowId};
#[cfg(test)]
pub(crate) use quick::tests::pin_quick_task;
pub(crate) use quick::{
    HOST_UNCHANGED, alpine_git_id, pin_project_free_quick_task_with_directories,
    pin_quick_task_with_context,
};
pub(crate) use resolve::{ResolvedEnvironmentSet, preview_environments, resolve_environments};
pub(crate) use run::{
    PhaseModelSelection, PinnedPreset, RunKind, RunSource, TaskSelection, WorkflowRun, now_ms,
};
pub(crate) use store::{RunSummary, WorkflowRunStore};
pub(crate) use task_loop::{
    LoopSummary, TaskListSnapshot, TaskLoop, TaskLoopError, TaskLoopItem, TaskLoopStore,
    TaskOutcome,
};
