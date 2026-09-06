#[cfg(test)]
pub(super) mod tests;

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
pub(crate) mod workspace;

pub(crate) use artefacts::WorkflowArtefactRepository;
pub(crate) use catalogue::{
    CatalogueError, ResolveWorkflowError, WorkflowCatalogue, WorkflowRecord, WorkflowSelection,
    definition_fits_agent,
};
pub(crate) use commit::CommitJournals;
pub(crate) use evidence::{AttemptEvidenceContext, WorkflowEvidenceStore};
pub(crate) use execution::{ExecutionGuard, WorkflowExecution};
pub(crate) use executor::{
    WorkflowContinuationRegistry, WorkflowJob, execute_run, interrupt_provider_continuations,
    interrupt_session_continuations, recover_commit_transactions, settle_cancelled_job,
    settle_terminal_job,
};
pub(crate) use id::{ArtefactId, AttemptId, GateId, RunId, WorkflowId};
#[cfg(test)]
pub(crate) use quick::tests::pin_quick_task;
pub(crate) use quick::{HOST_UNCHANGED, alpine_git_id, pin_quick_task_with_context};
pub(crate) use resolve::{preview_environments, resolve_environments};
pub(crate) use run::{PhaseModelSelection, PinnedPreset, RunKind, RunSource, WorkflowRun, now_ms};
pub(crate) use store::{RunSummary, WorkflowRunStore};
