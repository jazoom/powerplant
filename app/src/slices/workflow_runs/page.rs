use askama::Template;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::workflows::{RunSummary, WorkflowRun};

pub(super) const INDEX_TITLE: &str = "Runs | Power Plant";
pub(super) const DETAIL_TITLE: &str = "Run | Power Plant";

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
}

#[derive(Template)]
#[template(path = "workflow_runs/templates/index.html")]
pub(super) struct RunIndexView {
    pub(super) runs: Vec<IndexRow>,
}

pub(super) struct IndexRow {
    pub(super) id: String,
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
    pub(super) version: String,
    pub(super) state: &'static str,
    pub(super) created: String,
    pub(super) current_step: String,
    pub(super) steps: Vec<StepView>,
    pub(super) attempts: Vec<AttemptView>,
}

#[derive(Template)]
#[template(path = "workflow_runs/templates/detail.html", block = "run_detail")]
pub(super) struct RunDetailContents<'a> {
    pub(super) run_id: &'a str,
    pub(super) name: &'a str,
    pub(super) version: &'a str,
    pub(super) state: &'static str,
    pub(super) created: &'a str,
    pub(super) current_step: &'a str,
    pub(super) steps: &'a [StepView],
    pub(super) attempts: &'a [AttemptView],
}

impl RunDetailView {
    pub(super) fn from_run(run: &WorkflowRun) -> Self {
        Self {
            run_id: run.id.as_hex(),
            name: run.pinned.definition.name().to_owned(),
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
                })
                .collect(),
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
        }
    }

    pub(super) fn contents(&self) -> RunDetailContents<'_> {
        RunDetailContents {
            run_id: &self.run_id,
            name: &self.name,
            version: &self.version,
            state: self.state,
            created: &self.created,
            current_step: &self.current_step,
            steps: &self.steps,
            attempts: &self.attempts,
        }
    }
}

fn format_time(ms: u64) -> String {
    let seconds = i64::try_from(ms / 1000).unwrap_or(0);
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| time.format(&Rfc3339).ok())
        .unwrap_or_else(|| "unknown".to_owned())
}
