impl AppState {
    pub(crate) fn keep_temp_dir(&self, dir: tempfile::TempDir) {
        self.scratch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(dir);
    }
}

use super::*;

pub(crate) fn test_state(config: RuntimeConfig) -> AppState {
    let environments = Arc::new(EnvironmentCatalogue::in_memory());
    let environment_snapshots = Arc::new(EnvironmentSnapshotRepository::in_memory());
    let environment_preparations =
        EnvironmentPreparationScheduler::idle(environments.clone(), environment_snapshots.clone());
    AppState {
        config: Arc::new(config),
        assets: Arc::new(AssetPaths {
            css_path: "/static/test.css".to_owned(),
            js_path: "/static/test.js".to_owned(),
        }),
        sessions: Arc::new(SessionStore::new()),
        vault: Arc::new(ProviderVault::in_memory()),
        chat: Arc::new(ChatBackend::Scripted(
            crate::tests::ScriptedBackend::accept(),
        )),
        models: Arc::new(ModelCatalogue::default()),
        plan_login: Arc::new(PlanLogin::new()),
        preferences: Arc::new(Preferences::in_memory()),
        agents: Arc::new(AgentStore::in_memory()),
        projects: Arc::new(ProjectStore::in_memory()),
        sandboxes: Arc::new(SandboxFleet::scripted()),
        agent_leases: Arc::new(AgentLeaseCoordinator::new()),
        workflows: Arc::new(WorkflowCatalogue::in_memory()),
        workflow_runs: Arc::new(WorkflowRunStore::in_memory()),
        workflow_artefacts: Arc::new(WorkflowArtefactRepository::in_memory()),
        workflow_execution: Arc::new(WorkflowExecution::new()),
        gate_continuations: Arc::new(WorkflowContinuationRegistry::new()),
        workflow_workspaces: Arc::new(WorkflowWorkspaces::in_memory()),
        commit_journals: Arc::new(CommitJournals::in_memory()),
        environments,
        environment_snapshots,
        environment_preparations,
        scratch: Arc::new(std::sync::Mutex::new(Vec::new())),
    }
}

use std::collections::BTreeSet;

use super::recovered_cleanup_record;
use crate::workflows::run::AttemptCleanupRecord;
use crate::workflows::workspace::WorkspaceRecovery;
use crate::workflows::{AttemptId, RunId};

#[test]
fn startup_recovery_records_each_remaining_resource() {
    let run = RunId::generate().expect("run");
    let attempt = AttemptId::generate().expect("attempt");
    let workspace = WorkspaceRecovery {
        run,
        attempt,
        remains: true,
    };
    let mut guests = BTreeSet::new();

    let mut remaining_runs = BTreeSet::new();
    assert_eq!(
        recovered_cleanup_record(true, &guests, &remaining_runs, &[], run, attempt),
        AttemptCleanupRecord::Complete
    );
    assert_eq!(
        recovered_cleanup_record(true, &guests, &remaining_runs, &[workspace], run, attempt),
        AttemptCleanupRecord::Orphaned {
            sandbox: false,
            workspace: true,
            journal: false,
        }
    );
    guests.insert(attempt);
    assert_eq!(
        recovered_cleanup_record(true, &guests, &remaining_runs, &[workspace], run, attempt),
        AttemptCleanupRecord::Orphaned {
            sandbox: true,
            workspace: true,
            journal: false,
        }
    );
    remaining_runs.insert(run);
    assert_eq!(
        recovered_cleanup_record(true, &BTreeSet::new(), &remaining_runs, &[], run, attempt,),
        AttemptCleanupRecord::Orphaned {
            sandbox: true,
            workspace: false,
            journal: false,
        }
    );
    assert_eq!(
        recovered_cleanup_record(false, &BTreeSet::new(), &BTreeSet::new(), &[], run, attempt),
        AttemptCleanupRecord::Orphaned {
            sandbox: true,
            workspace: false,
            journal: false,
        }
    );
}
