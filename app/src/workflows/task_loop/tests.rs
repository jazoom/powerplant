use super::*;
use crate::agents::AgentId;
use crate::conversations::{ConversationId, DocumentId};
use crate::projects::ProjectId;
use crate::tests::test_environment_id;
use crate::workflows::definition::PinnedWorkflowDefinition;
use crate::workflows::run::{RunKind, RunSource};
use crate::workflows::seeds::ralph_task_loop_definition;
use crate::workflows::{ArtefactId, RunId};

impl TaskLoopStore {
    pub(crate) fn in_memory() -> Self {
        Self {
            dir: None,
            inner: Mutex::new(BTreeMap::new()),
        }
    }
}

fn snapshot() -> TaskListSnapshot {
    let markdown = "# Tasks\n\n- [ ] First task\n- [ ] Second task\n".to_owned();
    TaskListSnapshot {
        document_id: DocumentId::generate().expect("document"),
        revision: 1,
        content_hash: crate::workflows::artefacts::ObjectHash::of(markdown.as_bytes()).as_str(),
        markdown,
    }
}

pub(crate) fn loop_record() -> TaskLoop {
    let definition = ralph_task_loop_definition(test_environment_id());
    let list = snapshot();
    let parsed = crate::workflows::task_list::parse(&list.markdown).expect("tasks");
    TaskLoop::create(
        TaskLoopId::generate().expect("loop"),
        1,
        ConversationId::generate().expect("conversation"),
        ProjectId::parse(&"b".repeat(32)).expect("project"),
        AgentId::generate().expect("agent"),
        "Implement each remaining task.".to_owned(),
        PinnedWorkflowDefinition::pin(None, definition),
        Vec::new(),
        crate::tests::test_environment_set(&ralph_task_loop_definition(test_environment_id())),
        list,
        parsed
            .eligible_tasks()
            .map(|task| TaskLoopItem {
                index: task.index,
                markdown: task.markdown.clone(),
                child_id: None,
                outcome: TaskOutcome::Pending,
            })
            .collect(),
    )
    .expect("loop")
}

pub(crate) fn completed_child(
    parent: &TaskLoop,
    child_id: RunId,
    source: ArtefactReference,
) -> WorkflowRun {
    let task = parent
        .tasks
        .iter()
        .find(|task| task.child_id == Some(child_id))
        .expect("task");
    let mut run = parent
        .child_run(child_id, 2, task.index, task.markdown.clone())
        .expect("child");
    run.source = RunSource::Captured {
        source: crate::workflows::run::RunSourceState {
            initial: source.clone(),
            accepted: source.clone(),
            observed: crate::workflows::run::ObservedCandidate::Exact { artefact: source },
        },
    };
    run.state = RunState::Completed;
    run.attempts.push(crate::workflows::run::AttemptRecord {
        id: crate::workflows::AttemptId::generate().expect("attempt"),
        step: crate::workflows::definition::StepKey::parse("commit").expect("commit step"),
        ordinal: 1,
        action_kind: crate::workflows::run::ActionKind::SystemCommand,
        started_at_ms: 2,
        finished_at_ms: Some(3),
        state: crate::workflows::run::AttemptState::Completed,
        result: Some(crate::workflows::run::AttemptResult::Completed {
            outputs: Vec::new(),
        }),
        review_route: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        capabilities: crate::tests::test_agent_capabilities(),
        sandbox: crate::workflows::run::AttemptSandboxRecord {
            kind: crate::workflows::run::AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: parent.environments.steps[0].snapshot_digest.clone(),
        },
        initial_context: None,
        cleanup: crate::workflows::run::AttemptCleanupRecord::Complete,
        commit_transaction: None,
        commit_result: Some(super::super::commit::CommitResult {
            commit: "a".repeat(40),
        }),
    });
    run
}

pub(crate) fn source(label: u8) -> ArtefactReference {
    ArtefactReference {
        id: ArtefactId::generate().expect("artefact"),
        kind: crate::workflows::definition::ArtefactKind::CandidateRevision,
        artefact_hash: crate::workflows::artefacts::ArtefactHash::parse(&format!(
            "sha256:{}",
            crate::hex::encode(&[label; 32])
        ))
        .expect("hash"),
    }
}

#[test]
fn parent_child_identities_stay_distinct() {
    let store = TaskLoopStore::in_memory();
    let record = store.create(loop_record()).expect("create");
    let (parent, first_id, first_task) = store.reserve_next_child(&record.id, 0).expect("reserve");
    assert_eq!(first_task.index, 0);
    assert_eq!(parent.current_child(), Some(first_id));
    let child = parent
        .child_run(first_id, 2, first_task.index, first_task.markdown)
        .expect("child");
    assert_eq!(child.parent_loop, Some(parent.id));
    assert_eq!(child.conversation_id, Some(parent.conversation_id));
    assert_eq!(child.kind, RunKind::Configured);
    assert_eq!(
        child.task_selection.as_ref().map(|task| task.index),
        Some(0)
    );
    assert_ne!(child.id.as_hex(), parent.id.as_hex());
    store
        .mark_dispatched(&parent.id, first_id)
        .expect("dispatch");
    let runs = crate::workflows::WorkflowRunStore::in_memory();
    runs.create(child).expect("store child");
    assert!(runs.summaries().is_empty());
    assert!(runs.for_conversation(&parent.conversation_id).is_empty());
}

#[test]
fn parent_source_stays_distinct_from_later_task_bases() {
    let store = TaskLoopStore::in_memory();
    let record = store.create(loop_record()).expect("create");
    let (parent, first_id, _) = store.reserve_next_child(&record.id, 0).expect("first");
    store
        .mark_dispatched(&parent.id, first_id)
        .expect("dispatch");
    let original = source(1);
    store
        .record_original_source(&parent.id, original.clone())
        .expect("original");
    let first = completed_child(
        &store.get(&parent.id).expect("loop"),
        first_id,
        original.clone(),
    );
    let (parent, advance) = store.complete_child(&parent.id, &first).expect("complete");
    assert_eq!(advance, LoopAdvance::Next);
    assert_eq!(parent.original_source.as_ref(), Some(&original));
    let (parent, second_id, _) = store.reserve_next_child(&parent.id, 1).expect("second");
    let second = parent
        .child_run(second_id, 4, 1, parent.tasks[1].markdown.clone())
        .expect("second child");
    assert_ne!(second.id, first_id);
    assert_eq!(parent.original_source.as_ref(), Some(&original));
    assert!(matches!(second.source, RunSource::Pending));
    assert!(second.attempts.is_empty());
    let parent = store
        .record_original_source(&parent.id, source(2))
        .expect("later source capture");
    assert_eq!(parent.original_source, Some(original));
}

#[test]
fn duplicate_dispatch_is_rejected_without_a_second_child() {
    let store = TaskLoopStore::in_memory();
    let record = store.create(loop_record()).expect("create");
    let (parent, first_id, _) = store.reserve_next_child(&record.id, 0).expect("first");
    store
        .mark_dispatched(&parent.id, first_id)
        .expect("dispatch");
    assert_eq!(
        store.mark_dispatched(&parent.id, first_id).err(),
        Some(TaskLoopError::DuplicateDispatch)
    );
    assert_eq!(
        store.reserve_next_child(&parent.id, 0).err(),
        Some(TaskLoopError::DuplicateDispatch)
    );
    assert_eq!(store.get(&parent.id).expect("loop").tasks[1].child_id, None);
    let first = completed_child(&store.get(&parent.id).expect("loop"), first_id, source(1));
    store.complete_child(&parent.id, &first).expect("complete");
    assert_eq!(
        store.complete_child(&parent.id, &first).err(),
        Some(TaskLoopError::DuplicateDispatch)
    );
}

#[test]
fn a_bound_blocks_dispatch_durably() {
    let dir = tempfile::tempdir().expect("directory");
    let store = TaskLoopStore::open(dir.path().join("loops")).expect("store");
    let parent = store.create(loop_record()).expect("create");
    assert_eq!(
        store
            .reserve_next_child(&parent.id, MAXIMUM_LOOP_ATTEMPTS)
            .err(),
        Some(TaskLoopError::AttemptLimit)
    );
    let reopened = TaskLoopStore::open(dir.path().join("loops")).expect("reopen");
    let parent = reopened.get(&parent.id).expect("parent");
    assert_eq!(parent.state, TaskLoopState::Blocked);
    assert!(parent.tasks.iter().all(|task| task.child_id.is_none()));
}

#[test]
fn checked_tasks_keep_the_remaining_tasks_original_indices() {
    let mut parent = loop_record();
    parent.task_list.markdown = parent.task_list.markdown.replacen("[ ]", "[x]", 1);
    parent.task_list.content_hash =
        crate::workflows::artefacts::ObjectHash::of(parent.task_list.markdown.as_bytes()).as_str();
    parent.tasks.remove(0);
    let store = TaskLoopStore::in_memory();
    let parent = store
        .create(parent)
        .expect("checked tasks are context only");
    let (parent, child, task) = store.reserve_next_child(&parent.id, 0).expect("reserve");
    assert_eq!(task.index, 1);
    assert!(
        parent
            .child_run(child, 2, 0, task.markdown.clone())
            .is_err()
    );
    assert!(
        parent
            .child_run(child, 2, task.index, task.markdown)
            .is_ok()
    );
    let mut file = parent.to_file().expect("file");
    file.task_list.markdown.push_str("altered context");
    assert_eq!(
        TaskLoop::from_file(file).err(),
        Some(TaskLoopError::Corrupt)
    );
}

#[test]
fn a_substituted_child_cannot_advance_the_parent() {
    let store = TaskLoopStore::in_memory();
    let parent = store.create(loop_record()).expect("create");
    let (parent, id, _) = store.reserve_next_child(&parent.id, 0).expect("reserve");
    store.mark_dispatched(&parent.id, id).expect("dispatch");
    let mut child = completed_child(&parent, id, source(1));
    child.attempts[0].commit_result = None;
    assert_eq!(
        store.complete_child(&parent.id, &child).err(),
        Some(TaskLoopError::Conflict)
    );
    child = completed_child(&parent, id, source(1));
    child.task_selection.as_mut().expect("selection").index = 1;
    assert_eq!(
        store.complete_child(&parent.id, &child).err(),
        Some(TaskLoopError::Conflict)
    );
    assert_eq!(store.get(&parent.id).expect("parent").completed_count(), 0);
}
