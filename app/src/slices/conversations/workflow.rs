use askama::Template;
use axum::{
    Form,
    extract::{Path, Query, State},
    response::Response,
};
use hypergraft::{GraftRequest, PatchGraft, PatchStatus};
use serde::Deserialize;

use crate::{
    agents::AccessMode,
    conversations::{ConversationRecord, resolve_authority},
    error::{AppError, AppResult},
    projects::ProjectId,
    responses,
    sessions::RequiredSession,
    state::AppState,
    workflows::{self, ResolveWorkflowError, WorkflowJob, WorkflowRun, WorkflowSelection},
};

const TITLE_SUFFIX: &str = " | Power Plant";

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct WorkflowQuery {
    workflow: String,
    target: String,
    brief: String,
}

#[derive(Deserialize)]
pub(super) struct WorkflowLaunchForm {
    revision: String,
    workflow: String,
    brief: String,
    target: String,
    #[serde(default)]
    preview_workflow: String,
    #[serde(default)]
    preview_target: String,
}

struct WorkflowOption {
    token: String,
    name: String,
    summary: String,
    effects: String,
    selected: bool,
}

struct TargetOption {
    id: String,
    name: String,
    access: String,
    selected: bool,
}

#[derive(Template)]
#[template(path = "conversations/templates/workflow.html", block = "launch_page")]
struct WorkflowLaunchView {
    document_title: String,
    conversation_id: String,
    revision: String,
    brief: String,
    workflows: Vec<WorkflowOption>,
    targets: Vec<TargetOption>,
    model_summary: String,
    access_summary: String,
    environment_summary: String,
    error: &'static str,
}

#[derive(Template)]
#[template(path = "conversations/templates/workflow.html", block = "launch_form")]
struct WorkflowLaunchContents<'a> {
    conversation_id: &'a str,
    revision: &'a str,
    brief: &'a str,
    workflows: &'a [WorkflowOption],
    targets: &'a [TargetOption],
    model_summary: &'a str,
    access_summary: &'a str,
    environment_summary: &'a str,
    error: &'static str,
}

impl WorkflowLaunchView {
    fn contents(&self) -> WorkflowLaunchContents<'_> {
        WorkflowLaunchContents {
            conversation_id: &self.conversation_id,
            revision: &self.revision,
            brief: &self.brief,
            workflows: &self.workflows,
            targets: &self.targets,
            model_summary: &self.model_summary,
            access_summary: &self.access_summary,
            environment_summary: &self.environment_summary,
            error: self.error,
        }
    }
}

pub(super) async fn show(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Path(conversation_id): Path<String>,
    Query(query): Query<WorkflowQuery>,
) -> AppResult<Response> {
    let Some(record) = super::load_conversation(&state, &conversation_id) else {
        return Ok(responses::request_navigation(graft, "/conversations"));
    };
    let view = launch_view(
        &state,
        &record,
        if query.workflow.is_empty() {
            None
        } else {
            Some(query.workflow.as_str())
        },
        if query.target.is_empty() {
            None
        } else {
            Some(query.target.as_str())
        },
        &query.brief,
        "",
    )
    .await;
    render(graft, PatchStatus::Ok, &view, &state)
}

pub(super) async fn launch(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<WorkflowLaunchForm>,
) -> AppResult<Response> {
    let Some(record) = super::load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let error_view = |status, error| {
        let state = state.clone();
        let record = record.clone();
        let workflow = form.workflow.clone();
        let target = form.target.clone();
        let brief = form.brief.clone();
        async move {
            let view = launch_view(
                &state,
                &record,
                Some(workflow.as_str()),
                Some(target.as_str()),
                &brief,
                error,
            )
            .await;
            render(graft, status, &view, &state)
        }
    };
    let Some(revision) = super::parse_revision(&form.revision) else {
        return error_view(PatchStatus::UnprocessableEntity, super::REVISION_MESSAGE).await;
    };
    if record.revision != revision {
        return error_view(PatchStatus::Conflict, super::REVISION_MESSAGE).await;
    }
    if record.active_job.is_some() || state.sessions.busy(&session.0) {
        return error_view(
            PatchStatus::Conflict,
            "Wait until the current conversation command finishes.",
        )
        .await;
    }
    let brief = match workflows::input_context::validate_launch_brief(&form.brief) {
        Ok(brief) => brief,
        Err(error) => return error_view(PatchStatus::UnprocessableEntity, error.message()).await,
    };
    let Some(selection) = WorkflowSelection::parse(form.workflow.trim()) else {
        return error_view(
            PatchStatus::UnprocessableEntity,
            "Choose a current workflow from the catalogue.",
        )
        .await;
    };
    let resolved = match state.workflows.resolve(&selection) {
        Ok(resolved) => resolved,
        Err(error) => {
            let status = match error {
                ResolveWorkflowError::Missing | ResolveWorkflowError::Changed => {
                    PatchStatus::Conflict
                }
                ResolveWorkflowError::Invalid => PatchStatus::UnprocessableEntity,
            };
            return error_view(status, error.message()).await;
        }
    };
    if form.workflow != form.preview_workflow || form.target != form.preview_target {
        return error_view(
            PatchStatus::Conflict,
            "The selection changed. Review its access and environment readiness before launch.",
        )
        .await;
    }
    let run_id = workflows::RunId::generate()
        .map_err(|error| AppError::new("create workflow run identifier", error))?;
    let Some(target) = ProjectId::parse(form.target.trim()) else {
        return error_view(
            PatchStatus::UnprocessableEntity,
            "Choose an available target project.",
        )
        .await;
    };
    if !record.projects.contains(&target)
        || !record.grants.iter().any(|grant| grant.project_id == target)
    {
        return error_view(
            PatchStatus::UnprocessableEntity,
            "Grant access to the target project before launch.",
        )
        .await;
    }
    let mut target_record = record.clone();
    target_record.execution_target = Some(target);
    let authority = match resolve_authority(&target_record, &state.projects, &state.agents) {
        Ok(Some(authority)) => authority.effective,
        Ok(None) => {
            return error_view(
                PatchStatus::UnprocessableEntity,
                "Grant access to the target project before launch.",
            )
            .await;
        }
        Err(error) => return error_view(PatchStatus::Conflict, error.message()).await,
    };
    if !workflows::definition_fits_agent(
        &resolved.pinned.definition,
        &authority.tools,
        &authority
            .policy
            .grants()
            .iter()
            .map(|grant| (grant.alias.clone(), grant.access))
            .collect::<Vec<_>>(),
        &authority.grant_alias,
    ) {
        return error_view(
            PatchStatus::UnprocessableEntity,
            "That workflow needs access outside the conversation ceiling.",
        )
        .await;
    }
    let Some(model) = super::effective_model(&state, &record) else {
        return error_view(
            PatchStatus::UnprocessableEntity,
            "Choose a model before you launch a workflow.",
        )
        .await;
    };
    if let Err(error) = super::valid_selection(&state, &model.selection) {
        return error_view(PatchStatus::UnprocessableEntity, error).await;
    }
    let Some(connection) = state.vault.connection_for(&model.selection) else {
        return error_view(
            PatchStatus::UnprocessableEntity,
            "Choose a stored provider.",
        )
        .await;
    };
    let environments = match workflows::resolve_environments(
        &resolved.pinned.definition,
        &state.environments,
        &state.environment_snapshots,
    )
    .await
    {
        Ok(environments) => environments,
        Err(error) => return error_view(PatchStatus::UnprocessableEntity, error.message()).await,
    };
    let execution = match state.workflow_execution.acquire() {
        Ok(execution) => execution,
        Err(_) => {
            return error_view(
                PatchStatus::Conflict,
                "Wait until the current workflow finishes.",
            )
            .await;
        }
    };
    let current = if target_record.execution_target != record.execution_target {
        match state
            .conversations
            .select_execution_target(&record.id, revision, target)
        {
            Ok(current) => current,
            Err(error) => {
                return error_view(super::status_for(error), error.message()).await;
            }
        }
    } else {
        record.clone()
    };
    let authority = match resolve_authority(&current, &state.projects, &state.agents) {
        Ok(Some(authority)) => authority.effective,
        Ok(None) => {
            return error_view(
                PatchStatus::Conflict,
                "Project access was lost before launch.",
            )
            .await;
        }
        Err(error) => return error_view(PatchStatus::Conflict, error.message()).await,
    };
    let job = match state.sessions.begin_conversation_job(
        &session.0,
        current.id,
        current.messages.len() + 1,
    ) {
        Ok(job) => job,
        Err(_) => {
            return error_view(
                PatchStatus::Conflict,
                "Another command is active in this browser session.",
            )
            .await;
        }
    };
    let started = match state.conversations.begin_message_with_model(
        &current.id,
        current.revision,
        model,
        job.id(),
        brief.clone(),
    ) {
        Ok(started) => started,
        Err(error) => {
            state
                .sessions
                .finish_conversation_job(&session.0, current.id, job.id());
            return error_view(super::status_for(error), error.message()).await;
        }
    };
    let run = WorkflowRun::create_configured_for_conversation(
        run_id,
        workflows::now_ms(),
        authority.project_id,
        started.id,
        brief,
        resolved.pinned,
        environments,
    );
    if let Err(error) = state.workflow_runs.create(run.clone()) {
        let _ = state.conversations.settle_message(
            &started.id,
            job.id(),
            String::new(),
            crate::conversations::MessageStatus::Failed,
        );
        let _ = state
            .sessions
            .finish_conversation_job(&session.0, started.id, job.id());
        return Err(AppError::new("store workflow run", error));
    }
    job.set_workflow_name(run.pinned.definition.name().to_owned());
    job.set_step_label("Source capture".to_owned());
    tokio::spawn(workflows::execute_run(
        state.clone(),
        WorkflowJob {
            run_id,
            session_id: session.0,
            project_id: authority.project_id,
            agent_id: run.agent_id,
            agent_revision: authority.revision,
            conversation_id: Some(started.id),
            authority: Some(authority.clone()),
            grant_alias: authority.grant_alias.clone(),
            grant_access: authority.grant_access,
            connection,
            host_policy: authority.policy.clone(),
            turns: Vec::new(),
            job: job.clone(),
            eligible_reply: std::sync::Arc::new(std::sync::Mutex::new(String::new())),
        },
        None,
        execution,
    ));
    Ok(responses::command_navigation(&format!(
        "/conversations/{}",
        started.id.as_hex()
    )))
}

async fn launch_view(
    state: &AppState,
    record: &ConversationRecord,
    workflow_raw: Option<&str>,
    target_raw: Option<&str>,
    brief: &str,
    error: &'static str,
) -> WorkflowLaunchView {
    let records = state.workflows.list();
    let selected_workflow = selected_workflow(&records, workflow_raw);
    let workflows = records
        .iter()
        .map(|record| {
            let selection = WorkflowSelection {
                workflow_id: record.id,
                definition_version: record.definition_version,
            };
            WorkflowOption {
                token: selection.as_token(),
                name: record.definition.name().to_owned(),
                summary: workflow_summary(&record.definition),
                effects: workflow_effects(&record.definition),
                selected: selected_workflow == selection.as_token(),
            }
        })
        .collect();
    let selected_target = target_raw
        .and_then(|raw| ProjectId::parse(raw.trim()))
        .or(record.execution_target)
        .or_else(|| record.grants.first().map(|grant| grant.project_id));
    let targets = record
        .grants
        .iter()
        .filter_map(|grant| {
            let project = state.projects.get(&grant.project_id)?;
            Some(TargetOption {
                id: grant.project_id.as_hex(),
                name: project.name.clone(),
                access: target_access_summary(state, record, grant.project_id),
                selected: selected_target == Some(grant.project_id),
            })
        })
        .collect();
    let (model_summary, access_summary, environment_summary) =
        launch_readiness(state, record, selected_target, &selected_workflow).await;
    WorkflowLaunchView {
        document_title: format!("Run workflow · {}{}", record.title, TITLE_SUFFIX),
        conversation_id: record.id.as_hex(),
        revision: record.revision.to_string(),
        brief: brief.to_owned(),
        workflows,
        targets,
        model_summary,
        access_summary,
        environment_summary,
        error,
    }
}

async fn launch_readiness(
    state: &AppState,
    record: &ConversationRecord,
    target: Option<ProjectId>,
    workflow: &str,
) -> (String, String, String) {
    let model_summary = super::effective_model(state, record).map_or_else(
        || "No model selected".to_owned(),
        |model| {
            let effort = model
                .selection
                .thinking
                .as_ref()
                .map(|effort| format!(" · Thinking: {}", effort.label()))
                .unwrap_or_default();
            format!(
                "{} · {}{}",
                model.selection.provider.label(),
                model.selection.model,
                effort
            )
        },
    );
    let Some(target) = target else {
        return (
            model_summary,
            "No execution target. Grant project access first.".to_owned(),
            "Select a granted target to preview execution readiness.".to_owned(),
        );
    };
    let mut selected = record.clone();
    selected.execution_target = Some(target);
    let authority = match resolve_authority(&selected, &state.projects, &state.agents) {
        Ok(Some(authority)) => authority.effective,
        Ok(None) => {
            return (
                model_summary,
                "No execution authority".to_owned(),
                "The selected target has no grant.".to_owned(),
            );
        }
        Err(error) => {
            return (
                model_summary,
                error.message().to_owned(),
                "The selected target is not ready.".to_owned(),
            );
        }
    };
    let access_summary = format!(
        "{} · Tools: {} · Network: {}",
        access_label(authority.grant_access),
        authority
            .tools
            .iter()
            .map(|tool| tool.label())
            .collect::<Vec<_>>()
            .join(", "),
        network_label(&authority.network),
    );
    let Some(selection) = WorkflowSelection::parse(workflow) else {
        return (
            model_summary,
            access_summary,
            "Choose a workflow to check its environments.".to_owned(),
        );
    };
    let resolved = match state.workflows.resolve(&selection) {
        Ok(resolved) => resolved,
        Err(error) => return (model_summary, access_summary, error.message().to_owned()),
    };
    if !workflows::definition_fits_agent(
        &resolved.pinned.definition,
        &authority.tools,
        &authority
            .policy
            .grants()
            .iter()
            .map(|grant| (grant.alias.clone(), grant.access))
            .collect::<Vec<_>>(),
        &authority.grant_alias,
    ) {
        return (
            model_summary,
            access_summary,
            "The workflow needs access outside the effective ceiling.".to_owned(),
        );
    }
    match workflows::preview_environments(
        &resolved.pinned.definition,
        &state.environments,
        &state.environment_snapshots,
    )
    .await
    {
        Ok(preview) => {
            let names = preview
                .environments
                .iter()
                .map(|environment| environment.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            (model_summary, access_summary, format!("Ready: {names}"))
        }
        Err(error) => (model_summary, access_summary, error.message().to_owned()),
    }
}

fn selected_workflow(records: &[workflows::WorkflowRecord], raw: Option<&str>) -> String {
    if let Some(raw) = raw {
        return raw.trim().to_owned();
    }
    records
        .first()
        .map(|record| {
            WorkflowSelection {
                workflow_id: record.id,
                definition_version: record.definition_version,
            }
            .as_token()
        })
        .unwrap_or_default()
}

fn workflow_summary(definition: &workflows::definition::WorkflowDefinition) -> String {
    definition
        .steps()
        .iter()
        .map(|step| {
            let action = match &step.action {
                workflows::definition::StepAction::Agent(_) => "model phase",
                workflows::definition::StepAction::SystemCommand(_) => "system command",
                workflows::definition::StepAction::HumanGate(_) => "approval stop",
            };
            format!("{} ({action})", step.name)
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn workflow_effects(definition: &workflows::definition::WorkflowDefinition) -> String {
    if definition.steps().iter().any(|step| {
        matches!(&step.action, workflows::definition::StepAction::SystemCommand(action)
            if action.command == workflows::definition::SystemCommandId::CommitCandidate)
    }) {
        return "Can commit the candidate after its required approval".to_owned();
    }
    if definition
        .steps()
        .iter()
        .any(workflows::definition::StepDefinition::writes_primary_source)
    {
        "Can edit a candidate. Does not commit project changes".to_owned()
    } else {
        "Does not write the project".to_owned()
    }
}

fn target_access_summary(
    state: &AppState,
    record: &ConversationRecord,
    target: ProjectId,
) -> String {
    let mut selected = record.clone();
    selected.execution_target = Some(target);
    match resolve_authority(&selected, &state.projects, &state.agents) {
        Ok(Some(authority)) => format!(
            "{} · {} tools",
            access_label(authority.effective.grant_access),
            authority.effective.tools.len()
        ),
        Ok(None) => "No access".to_owned(),
        Err(error) => error.message().to_owned(),
    }
}

fn access_label(access: AccessMode) -> &'static str {
    match access {
        AccessMode::ReadOnly => "Read-only target",
        AccessMode::ReadWrite => "Writable target",
    }
}

fn network_label(network: &crate::agents::NetworkAccess) -> String {
    match network {
        crate::agents::NetworkAccess::None => "No network".to_owned(),
        crate::agents::NetworkAccess::Restricted(domains) => {
            format!("Restricted domains ({})", domains.len())
        }
        crate::agents::NetworkAccess::Public => "Public internet".to_owned(),
    }
}

fn render(
    graft: impl Into<GraftRequest>,
    status: PatchStatus,
    view: &WorkflowLaunchView,
    state: &AppState,
) -> AppResult<Response> {
    match graft.into() {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(&view.document_title, state, view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            &view.document_title,
            "chat-main",
            view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "workflow-launch",
            &view.contents(),
        )?),
    }
}

#[cfg(test)]
mod tests;
