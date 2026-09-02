use super::{CandidateDiff, DiffError, MANIFEST_PAGE_SIZE};
use crate::workflows::artefacts::candidate::{
    CandidateEntry, CandidateEntryKind, CandidateRevisionArtefact, GitAdministrativeFingerprint,
    GitObjectFormat, RepositoryAnchor, hash_entries,
};
use crate::workflows::artefacts::{
    ArtefactProducer, ArtefactProvenance, ArtefactRecord, ArtefactReference, ArtefactSummary,
    ObjectHash, ProductionDisposition, WorkflowArtefactRepository, artefact_hash_for,
};
use crate::workflows::definition::{ArtefactKind, PinnedWorkflowDefinition};

#[test]
fn manifest_pages_do_not_load_changed_blobs() {
    let store = WorkflowArtefactRepository::in_memory();
    let (run, base, target) = fixture(&store, 20, false);

    let diff = CandidateDiff::load(&run, &base, &target, &store).expect("diff manifests");
    let (total, page) = diff
        .manifest_page(0, MANIFEST_PAGE_SIZE)
        .expect("manifest page");

    assert_eq!(total, 20);
    assert_eq!(page.len(), MANIFEST_PAGE_SIZE);
    assert!(matches!(diff.change(0, &store), Err(DiffError::Missing)));
}

#[test]
fn selected_text_work_has_a_fixed_input_bound() {
    let store = WorkflowArtefactRepository::in_memory();
    let (run, base, target) = fixture(&store, 1, true);
    let diff = CandidateDiff::load(&run, &base, &target, &store).expect("diff manifests");

    let change = diff.change(0, &store).expect("selected change");

    assert!(change.text_too_large);
    assert!(change.text.is_none());
}

fn fixture(
    store: &WorkflowArtefactRepository,
    entries: usize,
    large: bool,
) -> (
    crate::workflows::WorkflowRun,
    ArtefactReference,
    ArtefactReference,
) {
    let definition = crate::tests::test_named_definition("Diff");
    let environments = crate::tests::test_environment_set(&definition);
    let mut run = crate::workflows::WorkflowRun::configured(
        crate::workflows::RunId::generate().expect("run"),
        1,
        crate::agents::AgentId::generate().expect("agent"),
        PinnedWorkflowDefinition::pin(None, definition),
        environments,
    );
    let base_candidate = candidate(Vec::new());
    let base_record = record(&run, &base_candidate, store, true);
    let base = reference(&base_record);
    run.record_initial_candidate(base_record).expect("base");

    let target_entries = (0..entries)
        .map(|index| {
            let bytes = if large && index == 0 { 300_000 } else { 10 };
            CandidateEntry {
                path: format!("file-{index:06}.txt"),
                kind: CandidateEntryKind::Regular {
                    executable: false,
                    bytes,
                    blob: ObjectHash::of(format!("missing-{index}").as_bytes()),
                },
            }
        })
        .collect();
    let target_candidate = candidate(target_entries);
    let target_record = record(&run, &target_candidate, store, false);
    let target = reference(&target_record);
    run.artefacts.push(target_record);
    (run, base, target)
}

fn candidate(entries: Vec<CandidateEntry>) -> CandidateRevisionArtefact {
    CandidateRevisionArtefact {
        format_version: crate::workflows::artefacts::candidate::CANDIDATE_SCHEMA,
        candidate_hash: hash_entries(&entries),
        repository: RepositoryAnchor {
            object_format: GitObjectFormat::Sha1,
            head: None,
        },
        git_admin: GitAdministrativeFingerprint::parse(&ObjectHash::of(b"git").as_str())
            .expect("git fingerprint"),
        entries,
    }
}

fn record(
    run: &crate::workflows::WorkflowRun,
    candidate: &CandidateRevisionArtefact,
    store: &WorkflowArtefactRepository,
    initial: bool,
) -> ArtefactRecord {
    let bytes = candidate.manifest_bytes().expect("manifest");
    let object_hash = store.publish(&bytes).expect("publish manifest");
    ArtefactRecord {
        id: crate::workflows::ArtefactId::generate().expect("artefact"),
        kind: ArtefactKind::CandidateRevision,
        artefact_hash: artefact_hash_for(ArtefactKind::CandidateRevision, 1, &bytes),
        object_hash,
        payload_bytes: bytes.len() as u64,
        created_at_ms: 1,
        provenance: ArtefactProvenance {
            run_id: run.id,
            producer: if initial {
                ArtefactProducer::RunSourceCapture
            } else {
                ArtefactProducer::StepAttempt {
                    attempt_id: crate::workflows::AttemptId::generate().expect("attempt"),
                    step: run.pinned.definition.first_step().clone(),
                    output: None,
                    disposition: ProductionDisposition::RequiredOutput,
                }
            },
            inputs: Vec::new(),
        },
        summary: ArtefactSummary::Candidate {
            candidate: candidate.candidate_hash,
            entries: candidate.entries.len() as u64,
            bytes: candidate.entries.len() as u64 * 10,
            disposition: ProductionDisposition::RequiredOutput,
        },
    }
}

fn reference(record: &ArtefactRecord) -> ArtefactReference {
    ArtefactReference {
        id: record.id,
        kind: record.kind,
        artefact_hash: record.artefact_hash,
    }
}
