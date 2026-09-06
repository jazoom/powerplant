use askama::Template;
use axum::{
    Form,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use hypergraft::{GraftRequest, PatchGraft, PatchStatus};
use serde::{Deserialize, Serialize};

use crate::{
    agents::{AccessMode, AgentId},
    conversations::ConversationRecord,
    error::{AppError, AppResult},
    projects::ProjectId,
    providers::{ModelSelection, ProviderKind, ThinkingEffort},
    responses,
    sessions::RequiredSession,
    state::AppState,
    workflows::{
        self, PhaseModelSelection, PinnedPreset, ResolveWorkflowError, WorkflowJob, WorkflowRun,
        WorkflowSelection,
    },
};

const TITLE_SUFFIX: &str = " | Power Plant";

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct WorkflowQuery {
    workflow: String,
    target: String,
    brief: String,
    #[serde(default)]
    phase: Vec<String>,
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
    #[serde(default)]
    phase: Vec<String>,
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

struct PhaseChoice {
    value: String,
    label: String,
    detail: String,
    selected: bool,
}

struct PhaseModelOption {
    step: String,
    name: String,
    choices: Vec<PhaseChoice>,
}

#[derive(Serialize, Deserialize)]
struct PhaseChoiceToken {
    step: String,
    provider: String,
    model: String,
    thinking: Option<String>,
    preset: Option<String>,
    preset_revision: Option<u32>,
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
    phase_models: Vec<PhaseModelOption>,
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
    phase_models: &'a [PhaseModelOption],
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
            phase_models: &self.phase_models,
            model_summary: &self.model_summary,
            access_summary: &self.access_summary,
            environment_summary: &self.environment_summary,
            error: self.error,
        }
    }
}

fn parse_fields<T: serde::de::DeserializeOwned>(
    fields: Vec<(String, String)>,
) -> Result<(T, Vec<String>), serde::de::value::Error> {
    let mut phases = Vec::new();
    let fields = fields.into_iter().filter(|(key, value)| {
        if key == "phase" {
            phases.push(value.clone());
            false
        } else {
            true
        }
    });
    let form = T::deserialize(serde::de::value::MapDeserializer::new(fields))?;
    Ok((form, phases))
}

pub(super) async fn show(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Path(conversation_id): Path<String>,
    Query(fields): Query<Vec<(String, String)>>,
) -> AppResult<Response> {
    let Ok((mut query, phases)) = parse_fields::<WorkflowQuery>(fields) else {
        return Ok(axum::http::StatusCode::BAD_REQUEST.into_response());
    };
    query.phase = phases;
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
        &query.phase,
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
    Form(fields): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    let Ok((mut form, phases)) = parse_fields::<WorkflowLaunchForm>(fields) else {
        return Ok(axum::http::StatusCode::UNPROCESSABLE_ENTITY.into_response());
    };
    form.phase = phases;
    let Some(record) = super::load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let error_view = |status, error| {
        let state = state.clone();
        let record = record.clone();
        let workflow = form.workflow.clone();
        let target = form.target.clone();
        let brief = form.brief.clone();
        let phase = form.phase.clone();
        async move {
            let view = launch_view(
                &state,
                &record,
                Some(workflow.as_str()),
                Some(target.as_str()),
                &brief,
                &phase,
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
    let authority = match crate::conversations::resolve_workflow_authority(
        &target_record,
        &state.projects,
        &state.agents,
    ) {
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
    let phase_models = match resolve_phase_models(&state, &resolved.pinned.definition, &form.phase)
    {
        Ok(models) => models,
        Err(error) => return error_view(PatchStatus::UnprocessableEntity, error).await,
    };
    if let Err(error) = validate_phase_models(
        &state,
        &resolved.pinned.definition,
        &authority,
        &phase_models,
    ) {
        return error_view(PatchStatus::UnprocessableEntity, error).await;
    }
    let selection = phase_models
        .first()
        .map(|phase| phase.selection.clone())
        .or_else(|| super::effective_model(&state, &record).map(|model| model.selection));
    let Some(connection) = selection
        .as_ref()
        .and_then(|selection| state.vault.connection_for(selection))
    else {
        return error_view(
            PatchStatus::UnprocessableEntity,
            "Choose a model before you launch a workflow.",
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
    let authority = match crate::conversations::resolve_workflow_authority(
        &current,
        &state.projects,
        &state.agents,
    ) {
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
        None,
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
        phase_models.clone(),
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
            phase_providers: phase_models
                .iter()
                .map(|phase| phase.selection.provider)
                .collect(),
            active_connection: std::sync::Arc::new(std::sync::Mutex::new(None)),
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
    phase_raw: &[String],
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
    let phase_models = selected_phase_model_options(state, record, &selected_workflow, phase_raw);
    WorkflowLaunchView {
        document_title: format!("Run workflow · {}{}", record.title, TITLE_SUFFIX),
        conversation_id: record.id.as_hex(),
        revision: record.revision.to_string(),
        brief: brief.to_owned(),
        workflows,
        targets,
        phase_models,
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
    let authority = match crate::conversations::resolve_workflow_authority(
        &selected,
        &state.projects,
        &state.agents,
    ) {
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

fn phase_choice_token(
    step: &str,
    selection: &ModelSelection,
    preset: Option<&crate::agents::AgentRecord>,
) -> String {
    serde_json::to_string(&PhaseChoiceToken {
        step: step.to_owned(),
        provider: selection.provider.as_str().to_owned(),
        model: selection.model.clone(),
        thinking: selection
            .thinking
            .as_ref()
            .map(|effort| effort.as_str().to_owned()),
        preset: preset.map(|record| record.id.as_hex()),
        preset_revision: preset.map(|record| record.revision),
    })
    .expect("phase model token")
}

fn parse_phase_choice(
    raw: &str,
    step: &str,
    agents: &[crate::agents::AgentRecord],
) -> Result<PhaseModelSelection, &'static str> {
    let token: PhaseChoiceToken =
        serde_json::from_str(raw).map_err(|_| "Choose a model for every model phase.")?;
    if token.step != step {
        return Err("Choose a model for every model phase.");
    }
    let provider = ProviderKind::parse(&token.provider)
        .ok_or("Choose an available provider for every model phase.")?;
    let thinking = token
        .thinking
        .map(|value| ThinkingEffort::new(value).ok_or("Choose an available thinking effort."))
        .transpose()?;
    let selection = ModelSelection::new(provider, token.model, thinking)
        .ok_or("Choose a valid model for every model phase.")?;
    let preset = match (token.preset, token.preset_revision) {
        (None, None) => None,
        (Some(id), Some(revision)) => {
            let id = AgentId::parse(&id).ok_or("Choose an available preset.")?;
            let record = agents
                .iter()
                .find(|record| record.id == id && record.revision == revision)
                .ok_or("That preset changed. Reload the launch sheet.")?;
            if let Some(preset_selection) = &record.selection
                && preset_selection != &selection
            {
                return Err("Use the model selected by that preset.");
            }
            Some(PinnedPreset {
                id: record.id,
                revision: record.revision,
                name: record.name.clone(),
            })
        }
        _ => return Err("Choose an available preset."),
    };
    let instructions = preset
        .as_ref()
        .and_then(|pinned| agents.iter().find(|record| record.id == pinned.id))
        .map(|record| record.instructions.clone())
        .unwrap_or_default();
    Ok(PhaseModelSelection {
        step: workflows::definition::StepKey::parse(step)
            .map_err(|_| "Choose a valid workflow phase.")?,
        selection,
        instructions,
        preset,
    })
}

fn phase_steps(
    definition: &workflows::definition::WorkflowDefinition,
) -> Vec<&workflows::definition::StepDefinition> {
    definition
        .steps()
        .iter()
        .filter(|step| matches!(&step.action, workflows::definition::StepAction::Agent(_)))
        .collect()
}

fn selected_phase_model_options(
    state: &AppState,
    record: &ConversationRecord,
    workflow_raw: &str,
    phase_raw: &[String],
) -> Vec<PhaseModelOption> {
    let Some(selection) = WorkflowSelection::parse(workflow_raw) else {
        return Vec::new();
    };
    let Ok(resolved) = state.workflows.resolve(&selection) else {
        return Vec::new();
    };
    let agents = state.agents.list();
    let selected = super::effective_model(state, record).map(|model| model.selection);
    let direct_models: Vec<ModelSelection> = state
        .vault
        .desk_providers()
        .into_iter()
        .filter_map(|provider| {
            let thinking = state.models_dev.effective_effort(
                provider.kind,
                &provider.model,
                provider.thinking.as_ref(),
            );
            ModelSelection::new(provider.kind, provider.model, thinking)
        })
        .chain(selected.clone())
        .fold(Vec::new(), |mut models, model| {
            if !models.contains(&model) {
                models.push(model);
            }
            models
        });
    phase_steps(&resolved.pinned.definition)
        .into_iter()
        .map(|step| {
            let mut choices = Vec::new();
            for selection in &direct_models {
                choices.push(PhaseChoice {
                    value: phase_choice_token(step.key.as_str(), selection, None),
                    label: format!(
                        "Direct model · {} · {}",
                        selection.provider.label(),
                        selection.model
                    ),
                    detail: selection
                        .thinking
                        .as_ref()
                        .map(|effort| format!("Thinking: {}", effort.label()))
                        .unwrap_or_else(|| {
                            "No preset instructions or extra authority ceiling".to_owned()
                        }),
                    selected: selected.as_ref() == Some(selection),
                });
            }
            let default_direct = direct_models.first();
            for agent in &agents {
                let Some(selection) = agent.selection.as_ref().or(default_direct) else {
                    continue;
                };
                choices.push(PhaseChoice {
                    value: phase_choice_token(step.key.as_str(), selection, Some(agent)),
                    label: format!("Preset · {}", agent.name),
                    detail: format!(
                        "{} · {}{}",
                        selection.provider.label(),
                        selection.model,
                        if agent.instructions.is_empty() {
                            String::new()
                        } else {
                            " · Saved instructions".to_owned()
                        }
                    ),
                    selected: false,
                });
            }
            let raw = phase_raw.iter().find(|raw| {
                serde_json::from_str::<PhaseChoiceToken>(raw)
                    .is_ok_and(|token| token.step == step.key.as_str())
            });
            if let Some(raw) = raw {
                for choice in &mut choices {
                    choice.selected = choice.value == *raw;
                }
                if !choices.iter().any(|choice| choice.selected) {
                    choices.push(PhaseChoice {
                        value: raw.clone(),
                        label: "Selected model or preset is unavailable".to_owned(),
                        detail: "Choose an available model or reload the launch sheet".to_owned(),
                        selected: true,
                    });
                }
            } else if !choices.iter().any(|choice| choice.selected)
                && let Some(first) = choices.first_mut()
            {
                first.selected = true;
            }
            PhaseModelOption {
                step: step.key.as_str().to_owned(),
                name: step.name.clone(),
                choices,
            }
        })
        .collect()
}

fn resolve_phase_models(
    state: &AppState,
    definition: &workflows::definition::WorkflowDefinition,
    phase_raw: &[String],
) -> Result<Vec<PhaseModelSelection>, &'static str> {
    let agents = state.agents.list();
    let mut models = Vec::new();
    for step in phase_steps(definition) {
        let mut matches = phase_raw.iter().filter(|raw| {
            serde_json::from_str::<PhaseChoiceToken>(raw)
                .is_ok_and(|token| token.step == step.key.as_str())
        });
        let Some(raw) = matches.next() else {
            return Err("Choose a model for every model phase.");
        };
        if matches.next().is_some() {
            return Err("Choose one model for every model phase.");
        }
        let model = parse_phase_choice(raw, step.key.as_str(), &agents)?;
        super::valid_selection(state, &model.selection)?;
        models.push(model);
    }
    if models.len() != phase_raw.len() {
        return Err("Choose a model only for the selected workflow phases.");
    }
    Ok(models)
}

fn validate_phase_models(
    state: &AppState,
    definition: &workflows::definition::WorkflowDefinition,
    base: &crate::agents::EffectiveAuthority,
    models: &[PhaseModelSelection],
) -> Result<(), &'static str> {
    for model in models {
        let step = definition
            .step(&model.step)
            .ok_or("Choose a valid workflow phase.")?;
        let authority = if let Some(preset) = &model.preset {
            let record = state
                .agents
                .get(&preset.id)
                .filter(|record| record.revision == preset.revision)
                .ok_or("That preset changed. Reload the launch sheet.")?;
            crate::conversations::apply_preset_ceiling(base, &record)
                .map_err(|error| error.message())?
        } else {
            base.clone()
        };
        let workflows::definition::StepAction::Agent(action) = &step.action else {
            return Err("Choose a model only for model phases.");
        };
        if action.candidate_authority.access().is_writable()
            && !authority.grant_access.is_writable()
            || !action
                .authority
                .allowed_by(&authority.tools, authority.directories())
        {
            return Err("That phase needs access outside the selected model ceiling.");
        }
    }
    Ok(())
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
    match crate::conversations::resolve_workflow_authority(
        &selected,
        &state.projects,
        &state.agents,
    ) {
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
