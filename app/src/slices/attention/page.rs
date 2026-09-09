use crate::{state::AppState, workflows::gates::HumanGateState};
use askama::Template;

#[derive(Template)]
#[template(path = "attention/templates/index.html")]
pub(crate) struct AttentionPage {
    decisions: Vec<Decision>,
    total: usize,
    previous: String,
    next: String,
}

struct Decision {
    title: String,
    context: String,
    href: String,
    evidence: String,
    reason: &'static str,
}

impl AttentionPage {
    pub(crate) fn count(state: &AppState) -> usize {
        decisions(state).len()
    }

    pub(super) fn new(state: &AppState, page: usize) -> Self {
        let decisions = decisions(state);
        let total = decisions.len();
        let page = page.min(total.saturating_sub(1) / 30);
        let next = if total > (page + 1) * 30 {
            format!("/attention?page={}", page + 1)
        } else {
            String::new()
        };
        let previous = if page > 0 {
            format!("/attention?page={}", page - 1)
        } else {
            String::new()
        };
        Self {
            decisions: decisions.into_iter().skip(page * 30).take(30).collect(),
            total,
            previous,
            next,
        }
    }
}

fn decisions(state: &AppState) -> Vec<Decision> {
    let mut decisions = Vec::new();
    let mut runs = state.workflow_runs.active_runs();
    runs.sort_by_key(|run| std::cmp::Reverse((run.created_at_ms, run.id)));
    for run in runs {
        let owner = run
            .conversation_id
            .and_then(|id| state.conversations.get(&id));
        for gate in run
            .gates
            .iter()
            .filter(|gate| gate.state == HumanGateState::AwaitingDecision)
        {
            let evidence = format!("/runs/{}/gates/{}", run.id.as_hex(), gate.id.as_hex());
            decisions.push(Decision {
                title: owner.as_ref().map_or_else(
                    || run.pinned.definition.name().to_owned(),
                    |record| record.title.clone(),
                ),
                context: run.pinned.definition.name().to_owned(),
                href: owner.as_ref().map_or_else(
                    || evidence.clone(),
                    |record| format!("/conversations/{}", record.id.as_hex()),
                ),
                evidence,
                reason: "Needs your review",
            });
        }
    }
    let mut conversations = state.conversations.list();
    conversations.sort_by_key(|record| std::cmp::Reverse(record.updated_at_ms));
    for record in conversations {
        if record
            .active_job
            .is_some_and(|job| state.host_approvals.pending_for(record.id, job).is_some())
        {
            decisions.push(Decision {
                title: record.title,
                context: "This computer · unrestricted host access".to_owned(),
                href: format!("/conversations/{}", record.id.as_hex()),
                evidence: String::new(),
                reason: "Needs command approval",
            });
        }
    }
    decisions
}
