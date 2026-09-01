use askama::Template;

use crate::workflows::artefacts::diff::{CandidateDiff, MANIFEST_PAGE_SIZE, TEXT_PAGE_FRAGMENTS};
use crate::workflows::gates::HumanGateRecord;
use crate::workflows::{RunKind, WorkflowRun};

pub(super) const TITLE: &str = "Human gate | Power Plant";

pub(super) struct ChangeRow {
    pub(super) path: String,
    pub(super) status: &'static str,
    pub(super) old: String,
    pub(super) new: String,
    pub(super) href: String,
    pub(super) base_download: String,
    pub(super) target_download: String,
}

pub(super) struct TextRow {
    pub(super) text: String,
    pub(super) range: String,
    pub(super) continued: bool,
}

#[derive(Template)]
#[template(path = "human_gates/templates/detail.html")]
pub(super) struct GatePage {
    pub(super) run_id: String,
    pub(super) gate_id: String,
    pub(super) gate_name: String,
    pub(super) position: usize,
    pub(super) state: &'static str,
    pub(super) base: String,
    pub(super) target: String,
    pub(super) revision: u64,
    pub(super) total: usize,
    pub(super) range: String,
    pub(super) changes: Vec<ChangeRow>,
    pub(super) previous: String,
    pub(super) next: String,
    pub(super) selected_path: String,
    pub(super) text: Vec<TextRow>,
    pub(super) text_previous: String,
    pub(super) text_next: String,
    pub(super) binary: bool,
    pub(super) text_too_large: bool,
    pub(super) awaiting: bool,
    pub(super) error: &'static str,
    pub(super) run_kind: &'static str,
    pub(super) project_id: String,
    pub(super) desk_href: String,
    pub(super) quick_task: bool,
}

impl GatePage {
    pub(super) fn new(
        run: &WorkflowRun,
        gate: &HumanGateRecord,
        diff: CandidateDiff,
        store: &crate::workflows::WorkflowArtefactRepository,
        query: super::forms::DiffQuery,
        error: &'static str,
    ) -> Option<Self> {
        let start = query.page.checked_mul(MANIFEST_PAGE_SIZE)?;
        let (total, page) = diff.manifest_page(start, MANIFEST_PAGE_SIZE).ok()?;
        let end = start + page.len();
        let root = format!("/runs/{}/gates/{}", run.id.as_hex(), gate.id.as_hex());
        let changes = page
            .iter()
            .enumerate()
            .map(|(offset, change)| {
                let index = start + offset;
                ChangeRow {
                    path: change.path.clone(),
                    status: change.status,
                    old: facts(change.old.as_ref()),
                    new: facts(change.new.as_ref()),
                    href: format!("{root}?page={}&change={index}", query.page),
                    base_download: download(&root, "base", index, change.old.as_ref()),
                    target_download: download(&root, "target", index, change.new.as_ref()),
                }
            })
            .collect();
        let mut selected_path = String::new();
        let mut text = Vec::new();
        let mut text_previous = String::new();
        let mut text_next = String::new();
        let mut binary = false;
        let mut text_too_large = false;
        if let Some(index) = query.change {
            let change = diff.change(index, store).ok()?;
            selected_path = change.path.clone();
            binary = change.binary;
            text_too_large = change.text_too_large;
            if let Some(fragments) = &change.text {
                let line_start = query.line.checked_mul(TEXT_PAGE_FRAGMENTS)?;
                if line_start > fragments.len() {
                    return None;
                }
                let line_end = (line_start + TEXT_PAGE_FRAGMENTS).min(fragments.len());
                text = fragments[line_start..line_end]
                    .iter()
                    .map(|fragment| TextRow {
                        text: fragment.text.clone(),
                        range: format!("bytes {}–{}", fragment.start, fragment.end),
                        continued: fragment.continued,
                    })
                    .collect();
                if query.line > 0 {
                    text_previous = format!(
                        "{root}?page={}&change={index}&line={}",
                        query.page,
                        query.line - 1
                    );
                }
                if line_end < fragments.len() {
                    text_next = format!(
                        "{root}?page={}&change={index}&line={}",
                        query.page,
                        query.line + 1
                    );
                }
            }
        }
        let previous = if query.page > 0 {
            format!("{root}?page={}", query.page - 1)
        } else {
            String::new()
        };
        let next = if end < total {
            format!("{root}?page={}", query.page + 1)
        } else {
            String::new()
        };
        Some(Self {
            run_id: run.id.as_hex(),
            gate_id: gate.id.as_hex(),
            gate_name: run.pinned.definition.step(&gate.step)?.name.clone(),
            position: run
                .pinned
                .definition
                .steps()
                .iter()
                .position(|step| step.key == gate.step)?
                + 1,
            state: state_label(gate.state),
            base: diff.base.as_str(),
            target: diff.target.as_str(),
            revision: gate.revision.get(),
            total,
            range: if start == end {
                "No changed paths".to_owned()
            } else {
                format!("Paths {}–{}", start + 1, end)
            },
            changes,
            previous,
            next,
            selected_path,
            text,
            text_previous,
            text_next,
            binary,
            text_too_large,
            awaiting: gate.state == crate::workflows::gates::HumanGateState::AwaitingDecision,
            error,
            run_kind: run.kind.as_str(),
            project_id: run.project_id.as_hex(),
            desk_href: crate::projects::desk_path(&run.project_id, &run.agent_id),
            quick_task: run.kind == RunKind::QuickTask,
        })
    }
}

fn facts(value: Option<&crate::workflows::artefacts::diff::EntryFacts>) -> String {
    value
        .map(|facts| {
            let mode = if facts.executable {
                "executable"
            } else {
                "not executable"
            };
            let bytes = facts
                .bytes
                .map(|bytes| format!(" · {bytes} bytes"))
                .unwrap_or_default();
            let object = facts
                .object
                .map(|hash| format!(" · {}", hash.as_str()))
                .unwrap_or_default();
            let detail = if facts.detail.is_empty() {
                String::new()
            } else {
                format!(" · {}", facts.detail)
            };
            format!("{} · {mode}{bytes}{object}{detail}", facts.kind)
        })
        .unwrap_or_else(|| "Absent".to_owned())
}

fn download(
    root: &str,
    side: &str,
    index: usize,
    value: Option<&crate::workflows::artefacts::diff::EntryFacts>,
) -> String {
    value
        .filter(|facts| facts.object.is_some())
        .map(|_| format!("{root}/objects/{side}/{index}"))
        .unwrap_or_default()
}

fn state_label(state: crate::workflows::gates::HumanGateState) -> &'static str {
    match state {
        crate::workflows::gates::HumanGateState::AwaitingDecision => "Awaiting decision",
        crate::workflows::gates::HumanGateState::Approved => "Approved",
        crate::workflows::gates::HumanGateState::RevisionRequested => "Revision requested",
        crate::workflows::gates::HumanGateState::Cancelled => "Cancelled",
        crate::workflows::gates::HumanGateState::Interrupted => "Interrupted",
    }
}
