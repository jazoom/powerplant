use super::{
    CommitError, NON_APPROVED_MESSAGE, commit_tree_command, hash_object_command,
    index_info_command, parse_object_id, require_approved_review, utc_timestamp,
    write_tree_command,
};
use crate::workflows::artefacts::candidate::GitObjectFormat;
use crate::workflows::artefacts::{
    ArtefactProducer, ArtefactProvenance, ArtefactRecord, ArtefactReference, ArtefactSummary,
    ProductionDisposition, ReviewVerdict, WorkflowArtefactRepository, artefact_hash_for, payload,
};
use crate::workflows::definition::{
    ArtefactKind, OutputKey, PinnedWorkflowDefinition, StepKey, test_environment_id,
};
use crate::workflows::id::{ArtefactId, AttemptId, RunId};
use crate::workflows::run::{
    AttemptArtefactInput, ObservedCandidate, RunSource, RunSourceState, WorkflowRun,
};
use crate::workflows::seeds::sequential_team_definition;

fn store() -> WorkflowArtefactRepository {
    WorkflowArtefactRepository::in_memory()
}

#[test]
fn journal_paths_are_derived_from_identifiers() {
    let dir = tempfile::tempdir().expect("dir");
    let journals =
        super::journal::CommitJournals::open(dir.path().join("workflow-commit-journals"))
            .expect("open");
    let run = RunId::generate().expect("run");
    let attempt = AttemptId::generate().expect("attempt");
    let journal = journals.create(run, attempt).expect("create");
    journal
        .write_index_backup("original.index", b"idx")
        .expect("write");
    journal.flush().expect("flush");
    assert_eq!(
        journal.read_index_backup("original.index").expect("read"),
        b"idx"
    );
    assert!(
        journals
            .path(run, attempt)
            .expect("derived")
            .ends_with(attempt.as_hex())
    );
    assert!(journals.load(run, attempt).is_ok());
    journals.remove(run, attempt).expect("remove");
    assert!(journals.load(run, attempt).is_err());
}

#[test]
fn command_contract_rejects_non_approved_and_stale_reviews() {
    let store = store();
    let definition = sequential_team_definition(test_environment_id());
    let environments = crate::workflows::test_environment_set(&definition);
    let mut run = WorkflowRun::create(
        RunId::generate().expect("run"),
        1,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let dir = tempfile::tempdir().expect("git");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .expect("git")
            .success()
    );
    let captured = crate::workflows::artefacts::CandidateCapture::capture_host(dir.path(), &store)
        .expect("capture");
    let bytes = captured.manifest_bytes().expect("manifest");
    let object = store.publish(&bytes).expect("object");
    let candidate = ArtefactRecord {
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
            producer: ArtefactProducer::StepAttempt {
                attempt_id: AttemptId::generate().expect("attempt"),
                step: StepKey::parse("implementer").expect("step"),
                output: Some(OutputKey::parse("candidate").expect("output")),
                disposition: ProductionDisposition::RequiredOutput,
            },
            inputs: Vec::new(),
        },
        summary: ArtefactSummary::Candidate {
            candidate: captured.candidate_hash,
            entries: 0,
            bytes: 0,
            disposition: ProductionDisposition::RequiredOutput,
        },
    };
    let reference = ArtefactReference {
        id: candidate.id,
        kind: candidate.kind,
        artefact_hash: candidate.artefact_hash,
    };
    run.artefacts.push(candidate.clone());
    run.source = RunSource::Captured {
        source: RunSourceState {
            initial: reference.clone(),
            accepted: reference.clone(),
            observed: ObservedCandidate::Exact {
                artefact: reference.clone(),
            },
        },
    };

    let (bytes, object, hash) = payload::encode_review(
        captured.candidate_hash,
        ReviewVerdict::RevisionRequired,
        "needs work",
        None,
    )
    .expect("review");
    store.publish(&bytes).expect("store");
    let review = ArtefactRecord {
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
            inputs: vec![reference.clone()],
        },
        summary: ArtefactSummary::Review {
            candidate: captured.candidate_hash,
            verdict: ReviewVerdict::RevisionRequired,
        },
    };
    run.artefacts.push(review.clone());
    let mut inputs = vec![
        AttemptArtefactInput {
            key: crate::workflows::definition::InputKey::parse("candidate").expect("key"),
            artefact: reference,
        },
        AttemptArtefactInput {
            key: crate::workflows::definition::InputKey::parse("review").expect("key"),
            artefact: ArtefactReference {
                id: review.id,
                kind: review.kind,
                artefact_hash: review.artefact_hash,
            },
        },
    ];
    assert_eq!(
        require_approved_review(&run, &inputs, &store).err(),
        Some(CommitError::Assurance)
    );
    assert_eq!(CommitError::Assurance.message(), NON_APPROVED_MESSAGE);

    for (bound, accepted) in [
        (captured.candidate_hash, true),
        (
            crate::workflows::artefacts::CandidateHash::of(b"stale"),
            false,
        ),
    ] {
        let (bytes, object, hash) =
            payload::encode_review(bound, ReviewVerdict::Approved, "approved", None)
                .expect("approved review");
        store.publish(&bytes).expect("store approved review");
        let stored_review = run.artefacts.last_mut().expect("review record");
        stored_review.object_hash = object;
        stored_review.artefact_hash = hash;
        stored_review.summary = ArtefactSummary::Review {
            candidate: bound,
            verdict: ReviewVerdict::Approved,
        };
        inputs[1].artefact.artefact_hash = hash;
        assert_eq!(
            require_approved_review(&run, &inputs, &store).is_ok(),
            accepted
        );
    }
}

fn run_git(project: &std::path::Path, exec: crate::sandbox::GuestExec) -> String {
    use std::io::Write;
    use std::process::Stdio;

    assert_eq!(exec.program, "git");
    let mut command = std::process::Command::new(&exec.program);
    command.current_dir(project).args(&exec.args).envs(exec.env);
    if exec.stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn git");
    if let Some(stdin) = exec.stdin {
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(&stdin)
            .expect("write stdin");
    }
    let output = child.wait_with_output().expect("git output");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_owned()
}

#[test]
fn fixed_plumbing_creates_the_exact_tree_without_touching_the_live_index() {
    let dir = tempfile::tempdir().expect("project");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .expect("init")
            .success()
    );
    std::fs::write(dir.path().join("old.txt"), b"old").expect("old");
    assert!(
        std::process::Command::new("git")
            .args(["add", "old.txt"])
            .current_dir(dir.path())
            .status()
            .expect("add")
            .success()
    );
    let live_index = std::fs::read(dir.path().join(".git/index")).expect("live index");
    let timestamp = utc_timestamp(1_700_000_000_000);
    let temporary_index = dir.path().join(".git/test-commit.index");
    let temporary_index_text = temporary_index.to_string_lossy();

    run_git(
        dir.path(),
        super::read_tree_empty_command(&temporary_index_text, &timestamp),
    );
    let regular = run_git(
        dir.path(),
        hash_object_command(b"regular\n".to_vec(), &timestamp),
    );
    let executable = run_git(
        dir.path(),
        hash_object_command(b"#!/bin/sh\n".to_vec(), &timestamp),
    );
    let mut index = Vec::new();
    for (path, mode, object) in [
        ("file.txt", "100644", regular.as_str()),
        ("script", "100755", executable.as_str()),
    ] {
        index.extend(format!("{mode} {object}\t{path}\0").into_bytes());
    }
    run_git(
        dir.path(),
        index_info_command(index, &temporary_index_text, &timestamp),
    );
    let tree = run_git(
        dir.path(),
        write_tree_command(&temporary_index_text, &timestamp),
    );
    let commit = run_git(dir.path(), commit_tree_command(&tree, None, &timestamp));

    assert!(parse_object_id(&tree, GitObjectFormat::Sha1).is_ok());
    assert!(parse_object_id(&commit, GitObjectFormat::Sha1).is_ok());
    assert_eq!(
        std::fs::read(dir.path().join(".git/index")).expect("unchanged index"),
        live_index
    );
    let committed_tree = std::process::Command::new("git")
        .args(["rev-parse", &format!("{commit}^{{tree}}")])
        .current_dir(dir.path())
        .output()
        .expect("tree");
    assert_eq!(
        String::from_utf8(committed_tree.stdout)
            .expect("tree id")
            .trim(),
        tree
    );
    let names = std::process::Command::new("git")
        .args(["ls-tree", "--name-only", &commit])
        .current_dir(dir.path())
        .output()
        .expect("tree names");
    assert_eq!(
        String::from_utf8(names.stdout).expect("names"),
        "file.txt\nscript\n"
    );
}
