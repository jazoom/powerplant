use askama::Template;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::environments::EnvironmentCatalogue;
use crate::workflows::{RunSummary, WorkflowCatalogue, WorkflowRun};

pub(super) const INDEX_TITLE: &str = "Runs | Power Plant";
pub(super) const DETAIL_TITLE: &str = "Run | Power Plant";
pub(super) const ARTEFACT_TITLE: &str = "Artefact | Power Plant";

pub(super) struct AttemptView {
    pub(super) ordinal: u32,
    pub(super) step: String,
    pub(super) action: &'static str,
    pub(super) state: &'static str,
    pub(super) started: String,
    pub(super) finished: String,
    pub(super) result: String,
}

pub(super) struct StepView {
    pub(super) name: String,
    pub(super) action: &'static str,
    pub(super) environment: String,
}

pub(super) struct PinnedEnvironmentView {
    pub(super) name: String,
    pub(super) note: String,
    pub(super) preparation: String,
    pub(super) recipe: String,
    pub(super) snapshot: String,
    pub(super) image: String,
}

#[derive(Template)]
#[template(path = "workflow_runs/templates/index.html")]
pub(super) struct RunIndexView {
    pub(super) runs: Vec<IndexRow>,
}

pub(super) struct IndexRow {
    pub(super) id: String,
    pub(super) uncatalogued: bool,
    pub(super) name: String,
    pub(super) version: String,
    pub(super) state: String,
    pub(super) created: String,
    pub(super) current_step: String,
    pub(super) latest_attempt: String,
}

impl RunIndexView {
    pub(super) fn from_summaries(summaries: &[RunSummary]) -> Self {
        Self {
            runs: summaries
                .iter()
                .map(|summary| IndexRow {
                    id: summary.id.as_hex(),
                    uncatalogued: summary.workflow_id.is_none(),
                    name: summary.name.clone(),
                    version: summary.version.as_hex(),
                    state: summary.state.clone(),
                    created: format_time(summary.created_at_ms),
                    current_step: summary.current_step.clone(),
                    latest_attempt: summary.latest_attempt.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Template)]
#[template(path = "workflow_runs/templates/detail.html")]
pub(super) struct RunDetailView {
    pub(super) run_id: String,
    pub(super) name: String,
    pub(super) name_href: String,
    pub(super) catalogue_note: String,
    pub(super) version: String,
    pub(super) state: &'static str,
    pub(super) created: String,
    pub(super) current_step: String,
    pub(super) steps: Vec<StepView>,
    pub(super) environments: Vec<PinnedEnvironmentView>,
    pub(super) attempts: Vec<AttemptView>,
    pub(super) artefacts: Vec<ArtefactRow>,
}

pub(super) struct ArtefactRow {
    pub(super) href: String,
    pub(super) kind: &'static str,
    pub(super) hash: String,
    pub(super) producer: &'static str,
    pub(super) created: String,
    pub(super) status: &'static str,
}

#[derive(Template)]
#[template(path = "workflow_runs/templates/detail.html", block = "run_detail")]
pub(super) struct RunDetailContents<'a> {
    pub(super) run_id: &'a str,
    pub(super) name: &'a str,
    pub(super) name_href: &'a str,
    pub(super) catalogue_note: &'a str,
    pub(super) version: &'a str,
    pub(super) state: &'static str,
    pub(super) created: &'a str,
    pub(super) current_step: &'a str,
    pub(super) steps: &'a [StepView],
    pub(super) environments: &'a [PinnedEnvironmentView],
    pub(super) attempts: &'a [AttemptView],
    pub(super) artefacts: &'a [ArtefactRow],
}

impl RunDetailView {
    pub(super) fn from_run(
        run: &WorkflowRun,
        workflows: &WorkflowCatalogue,
        environments: &EnvironmentCatalogue,
    ) -> Self {
        let (name_href, catalogue_note) = catalogue_presentation(run, workflows);
        Self {
            run_id: run.id.as_hex(),
            name: run.pinned.definition.name().to_owned(),
            name_href,
            catalogue_note,
            version: run.pinned.version.as_hex(),
            state: run.state.as_label(),
            created: format_time(run.created_at_ms),
            current_step: run.current_step_name().unwrap_or("").to_owned(),
            steps: run
                .pinned
                .definition
                .steps()
                .iter()
                .map(|step| StepView {
                    name: step.name.clone(),
                    action: step.action.kind_label(),
                    environment: step_environment_label(run, step),
                })
                .collect(),
            environments: pinned_environments(run, environments),
            attempts: run
                .attempts
                .iter()
                .map(|attempt| AttemptView {
                    ordinal: attempt.ordinal,
                    step: attempt.step.as_str().to_owned(),
                    action: attempt.action_kind.as_label(),
                    state: attempt.state.as_label(),
                    started: format_time(attempt.started_at_ms),
                    finished: attempt.finished_at_ms.map(format_time).unwrap_or_default(),
                    result: attempt
                        .result
                        .as_ref()
                        .map(|result| result.as_label())
                        .unwrap_or_default(),
                })
                .collect(),
            artefacts: artefact_rows(run),
        }
    }

    pub(super) fn contents(&self) -> RunDetailContents<'_> {
        RunDetailContents {
            run_id: &self.run_id,
            name: &self.name,
            name_href: &self.name_href,
            catalogue_note: &self.catalogue_note,
            version: &self.version,
            state: self.state,
            created: &self.created,
            current_step: &self.current_step,
            steps: &self.steps,
            environments: &self.environments,
            attempts: &self.attempts,
            artefacts: &self.artefacts,
        }
    }
}

fn step_environment_label(
    run: &WorkflowRun,
    step: &crate::workflows::definition::StepDefinition,
) -> String {
    let set = &run.environments;
    let Some(binding) = set.steps.iter().find(|item| item.step == step.key) else {
        return String::new();
    };
    let Some(environment) = set
        .environments
        .iter()
        .find(|item| item.environment_id == binding.environment_id)
    else {
        return String::new();
    };
    format!(
        "{} · {}",
        environment.name,
        environment.snapshot.snapshot_digest.short_hex()
    )
}

fn pinned_environments(
    run: &WorkflowRun,
    catalogue: &EnvironmentCatalogue,
) -> Vec<PinnedEnvironmentView> {
    run.environments
        .environments
        .iter()
        .map(|environment| {
            let note = match catalogue.get(&environment.environment_id) {
                None => "Environment deleted from catalogue",
                Some(record) if record.ready_preparation != Some(environment.preparation_id) => {
                    "Earlier pinned preparation"
                }
                Some(_) => "",
            };
            PinnedEnvironmentView {
                name: environment.name.clone(),
                note: note.to_owned(),
                preparation: environment.preparation_id.as_hex(),
                recipe: environment.recipe_version.as_hex(),
                snapshot: environment.snapshot.snapshot_digest.as_str().to_owned(),
                image: environment
                    .snapshot
                    .image_manifest_digest
                    .as_str()
                    .to_owned(),
            }
        })
        .collect()
}

fn catalogue_presentation(run: &WorkflowRun, catalogue: &WorkflowCatalogue) -> (String, String) {
    let Some(id) = run.pinned.workflow_id else {
        return (String::new(), "Uncatalogued definition".to_owned());
    };
    match catalogue.get(&id) {
        None => (String::new(), "Workflow deleted from catalogue".to_owned()),
        Some(record) if record.definition_version != run.pinned.version => (
            format!("/workflows/{}/configuration", id.as_hex()),
            "Earlier pinned version".to_owned(),
        ),
        Some(_) => (
            format!("/workflows/{}/configuration", id.as_hex()),
            String::new(),
        ),
    }
}

fn artefact_rows(run: &WorkflowRun) -> Vec<ArtefactRow> {
    let observed = run.observed_candidate_hash();
    run.artefacts
        .iter()
        .map(|record| ArtefactRow {
            href: format!("/runs/{}/artefacts/{}", run.id.as_hex(), record.id.as_hex()),
            kind: record.kind.as_str(),
            hash: record.artefact_hash.short(),
            producer: record.provenance.producer.as_label(),
            created: format_time(record.created_at_ms),
            status: record.assurance_label(observed),
        })
        .collect()
}

#[derive(askama::Template)]
#[template(path = "workflow_runs/templates/artefact.html")]
pub(super) struct ArtefactView {
    pub(super) run_href: String,
    pub(super) kind: &'static str,
    pub(super) hash: String,
    pub(super) producer: &'static str,
    pub(super) created: String,
    pub(super) body: String,
    pub(super) constraint: String,
    pub(super) preview: String,
    pub(super) truncated: bool,
}

impl ArtefactView {
    pub(super) fn from_record(
        run: &WorkflowRun,
        record: &crate::workflows::artefacts::ArtefactRecord,
        state: &crate::state::AppState,
    ) -> Self {
        let body = match state.workflow_artefacts.get(&record.object_hash) {
            Ok(bytes) => artefact_body(record.kind, bytes),
            Err(_) => String::new(),
        };
        let constraint = record.constraint_label();
        let (preview, truncated) = candidate_preview(run, record, state);
        Self {
            run_href: format!("/runs/{}", run.id.as_hex()),
            kind: record.kind.as_str(),
            hash: record.artefact_hash.as_str(),
            producer: record.provenance.producer.as_label(),
            created: format_time(record.created_at_ms),
            body,
            constraint,
            preview,
            truncated,
        }
    }
}

fn artefact_body(kind: crate::workflows::definition::ArtefactKind, bytes: Vec<u8>) -> String {
    match crate::workflows::artefacts::parse_typed_payload(kind, &bytes) {
        Ok(crate::workflows::artefacts::TypedPayload::Plan(plan)) => {
            crate::markdown::render(&plan.markdown)
        }
        Ok(crate::workflows::artefacts::TypedPayload::Review(report)) => {
            crate::markdown::render(&report.markdown)
        }
        Ok(crate::workflows::artefacts::TypedPayload::Test(report)) => {
            crate::markdown::render(&report.markdown)
        }
        Err(_) => match String::from_utf8(bytes) {
            Ok(text) => crate::markdown::escape_plain(&text),
            Err(_) => "Binary candidate manifest".to_owned(),
        },
    }
}

fn candidate_preview(
    run: &WorkflowRun,
    record: &crate::workflows::artefacts::ArtefactRecord,
    state: &crate::state::AppState,
) -> (String, bool) {
    if record.kind != crate::workflows::definition::ArtefactKind::CandidateRevision {
        return (String::new(), false);
    }
    let Ok(after_bytes) = state.workflow_artefacts.get(&record.object_hash) else {
        return (String::new(), false);
    };
    let Some(after) =
        crate::workflows::artefacts::candidate::CandidateRevisionArtefact::from_manifest_bytes(
            &after_bytes,
        )
    else {
        return (String::new(), false);
    };
    let before = record
        .provenance
        .inputs
        .iter()
        .find_map(|input| {
            if input.kind != crate::workflows::definition::ArtefactKind::CandidateRevision {
                return None;
            }
            let parent = run.artefact(&input.id)?;
            let bytes = state.workflow_artefacts.get(&parent.object_hash).ok()?;
            crate::workflows::artefacts::candidate::CandidateRevisionArtefact::from_manifest_bytes(
                &bytes,
            )
        })
        .unwrap_or_default();
    let (text, truncated) = crate::workflows::artefacts::candidate::preview_plain(
        &state.workflow_artefacts,
        &before,
        &after,
    );
    (crate::markdown::escape_plain(&text), truncated)
}

fn format_time(ms: u64) -> String {
    let seconds = i64::try_from(ms / 1000).unwrap_or(0);
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| time.format(&Rfc3339).ok())
        .unwrap_or_else(|| "unknown".to_owned())
}
