use axum::{
    Form,
    extract::{Path, Query, State},
    response::Response,
};
use hypergraft::{CommandGraft, GraftRequest, PatchStatus};

use crate::{
    agents::{AgentRecord, DirectoryPolicy},
    error::AppResult,
    projects::{EligibleGrant, ProjectId, ProjectRecord, desk_path, eligibility, eligible_agents},
    responses,
    sessions::{self, BeginTurnError, ConversationKey, JobIdError, SessionId, SessionSnapshot},
    state::AppState,
    workflows::{
        self, ResolveWorkflowError, RunKind, WorkflowJob, WorkflowRun, definition_fits_agent,
    },
};

use crate::slices::chat::{
    ChatForm, DeskMode, DeskPage, ObserveQuery, accept_job_patch, navigate_page, observe,
    reject_chat_input, reject_chat_selection, reject_parallel_command, render_document, view,
};

#[derive(Default, serde::Deserialize)]
pub(super) struct SendQuery {
    #[serde(default)]
    workflow: String,
}

struct DeskContext {
    session: SessionId,
    project: ProjectRecord,
    agent: AgentRecord,
    grant: EligibleGrant,
    snapshot: SessionSnapshot,
    key: ConversationKey,
    eligible: Vec<AgentRecord>,
}

impl DeskContext {
    fn page(&self) -> DeskPage<'_> {
        DeskPage {
            project: &self.project,
            agent: &self.agent,
            eligible: &self.eligible,
            snapshot: &self.snapshot,
        }
    }
}

pub(super) async fn show(
    State(state): State<AppState>,
    session: crate::sessions::RequiredSession,
    graft: GraftRequest,
    Path((project_id, agent_id)): Path<(String, String)>,
    Query(query): Query<ObserveQuery>,
) -> AppResult<Response> {
    let desk = match require_desk(&state, session.0, &project_id, &agent_id, graft).await {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    state
        .sessions
        .remember_conversation(&desk.session, desk.key);
    match graft {
        GraftRequest::Document => render_document(
            &state,
            PatchStatus::Ok,
            view(&state, desk.page(), "", "", &query.workflow).await,
        ),
        GraftRequest::Navigation => navigate_page(
            &state,
            &view(&state, desk.page(), "", "", &query.workflow).await,
        ),
        GraftRequest::Patch => observe(&state, &desk.session, desk.page(), query).await,
    }
}

pub(super) async fn send(
    State(state): State<AppState>,
    session: crate::sessions::RequiredSession,
    graft: CommandGraft,
    Path((project_id, agent_id)): Path<(String, String)>,
    Query(query): Query<SendQuery>,
    Form(form): Form<ChatForm>,
) -> AppResult<Response> {
    let desk = match require_desk(&state, session.0, &project_id, &agent_id, graft).await {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    let Some(connection) = state.vault.selected_connection() else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };

    if !form.is_bounded() {
        return reject_chat_input(
            &state,
            graft,
            desk.page(),
            "Enter a message.",
            &form.message,
        )
        .await;
    }
    let mode = match form.mode() {
        Ok(mode) => mode,
        Err(_) => {
            return reject_chat_input(
                &state,
                graft,
                desk.page(),
                "Choose a run mode.",
                &form.message,
            )
            .await;
        }
    };
    let configured_selection = if mode == DeskMode::Configured {
        match form.workflow_selection(&query.workflow) {
            Ok(selection) => Some(selection),
            Err(_) => {
                return reject_chat_input(
                    &state,
                    graft,
                    desk.page(),
                    "Choose a workflow.",
                    &form.message,
                )
                .await;
            }
        }
    } else {
        None
    };
    if desk.snapshot.session_busy {
        return reject_parallel_command(&state, graft, desk.page()).await;
    }
    if !desk.project.host_path_is_available() {
        return reject_chat_input(
            &state,
            graft,
            desk.page(),
            "A granted directory is no longer at the saved path.",
            &form.message,
        )
        .await;
    }

    let Ok(execution) = state.workflow_execution.acquire() else {
        return reject_chat_input(
            &state,
            graft,
            desk.page(),
            "Wait until the current workflow finishes.",
            &form.message,
        )
        .await;
    };
    let Ok(lease) = state.agent_leases.acquire(desk.agent.id) else {
        return reject_chat_input(
            &state,
            graft,
            desk.page(),
            "Wait until this agent finishes.",
            &form.message,
        )
        .await;
    };
    let Some(latest_agent) = state.agents.get(&desk.agent.id) else {
        return Ok(responses::graft_redirect(graft, "/projects"));
    };
    let Some(latest_project) = state.projects.get(&desk.project.id) else {
        return Ok(responses::graft_redirect(graft, "/projects"));
    };
    let Some(grant) = eligibility(&latest_agent, &latest_project) else {
        return Ok(responses::graft_redirect(
            graft,
            &format!("/projects/{}", latest_project.id.as_hex()),
        ));
    };
    if latest_agent.revision != desk.agent.revision {
        let eligible = eligible_agents(&state.agents.list(), &latest_project);
        return reject_chat_input(
            &state,
            graft,
            DeskPage {
                project: &latest_project,
                agent: &latest_agent,
                eligible: &eligible,
                snapshot: &desk.snapshot,
            },
            "The agent configuration changed. Try again.",
            &form.message,
        )
        .await;
    }
    if grant.alias != desk.grant.alias || grant.access != desk.grant.access {
        return Ok(responses::graft_redirect(
            graft,
            &format!("/projects/{}", latest_project.id.as_hex()),
        ));
    }
    let record = latest_agent;
    let project = latest_project;
    let eligible = eligible_agents(&state.agents.list(), &project);
    let policy = DirectoryPolicy::from_record_with_primary(&record, &grant.alias);
    if policy.confirm_hosts().is_err() {
        return reject_chat_input(
            &state,
            graft,
            DeskPage {
                project: &project,
                agent: &record,
                eligible: &eligible,
                snapshot: &desk.snapshot,
            },
            "A granted directory is no longer at the saved path.",
            &form.message,
        )
        .await;
    }
    let directories: Vec<(String, crate::agents::AccessMode)> = record
        .directories
        .iter()
        .map(|grant| (grant.alias.clone(), grant.access))
        .collect();
    let latest_page = DeskPage {
        project: &project,
        agent: &record,
        eligible: &eligible,
        snapshot: &desk.snapshot,
    };
    let (kind, pinned, environments) = match configured_selection {
        Some(ref selection) => {
            let resolved = match state.workflows.resolve(selection) {
                Ok(resolved) => resolved,
                Err(error) => {
                    let status = match error {
                        ResolveWorkflowError::Missing | ResolveWorkflowError::Changed => {
                            PatchStatus::Conflict
                        }
                        ResolveWorkflowError::Invalid => PatchStatus::UnprocessableEntity,
                    };
                    return reject_chat_selection(
                        &state,
                        graft,
                        latest_page,
                        error.message(),
                        &form.message,
                        status,
                    )
                    .await;
                }
            };
            if !definition_fits_agent(
                &resolved.pinned.definition,
                &record.tools,
                &directories,
                &grant.alias,
            ) {
                return reject_chat_input(
                    &state,
                    graft,
                    latest_page,
                    "That workflow needs access this agent does not allow.",
                    &form.message,
                )
                .await;
            }
            let environments = match workflows::resolve_environments(
                &resolved.pinned.definition,
                &state.environments,
                &state.environment_snapshots,
            )
            .await
            {
                Ok(environments) => environments,
                Err(error) => {
                    return reject_chat_input(
                        &state,
                        graft,
                        latest_page,
                        error.message(),
                        &form.message,
                    )
                    .await;
                }
            };
            (RunKind::Configured, resolved.pinned, environments)
        }
        None => {
            let environment_id = match workflows::alpine_git_id(&state.environments) {
                Ok(id) => id,
                Err(error) => {
                    return reject_chat_input(
                        &state,
                        graft,
                        latest_page,
                        error.message(),
                        &form.message,
                    )
                    .await;
                }
            };
            let pinned = match workflows::pin_quick_task(
                grant.access,
                &record.tools,
                &record.instructions,
                environment_id,
            ) {
                Ok(pinned) => pinned,
                Err(error) => {
                    return reject_chat_input(
                        &state,
                        graft,
                        latest_page,
                        error.message(),
                        &form.message,
                    )
                    .await;
                }
            };
            if !definition_fits_agent(
                &pinned.definition,
                &record.tools,
                &directories,
                &grant.alias,
            ) {
                return reject_chat_input(
                    &state,
                    graft,
                    latest_page,
                    "That workflow needs access this agent does not allow.",
                    &form.message,
                )
                .await;
            }
            let environments = match workflows::resolve_environments(
                &pinned.definition,
                &state.environments,
                &state.environment_snapshots,
            )
            .await
            {
                Ok(environments) => environments,
                Err(error) => {
                    return reject_chat_input(
                        &state,
                        graft,
                        latest_page,
                        error.message(),
                        &form.message,
                    )
                    .await;
                }
            };
            (RunKind::QuickTask, pinned, environments)
        }
    };

    let message = form.message.trim().to_owned();
    let run_id = workflows::RunId::generate()
        .map_err(|error| crate::error::AppError::new("create workflow run identifier", error))?;
    let workflow_name = pinned.definition.name().to_owned();
    let key = ConversationKey {
        project_id: project.id,
        agent_id: record.id,
    };
    let started = match state
        .sessions
        .begin_turn(&desk.session, key, run_id, message)
    {
        Ok(started) => started,
        Err(BeginTurnError::MissingSession) => {
            return Ok(responses::graft_redirect(graft, "/connect"));
        }
        Err(BeginTurnError::Conflict) => {
            let Some(latest) = state.sessions.snapshot(&desk.session, &key) else {
                return Ok(responses::graft_redirect(graft, "/connect"));
            };
            return reject_parallel_command(
                &state,
                graft,
                DeskPage {
                    project: &project,
                    agent: &record,
                    eligible: &eligible,
                    snapshot: &latest,
                },
            )
            .await;
        }
        Err(BeginTurnError::JobId) => {
            return Err(crate::error::AppError::new(
                "create job identifier",
                JobIdError::RandomUnavailable,
            ));
        }
    };
    let run = WorkflowRun::create(
        run_id,
        workflows::now_ms(),
        project.id,
        record.id,
        kind,
        pinned,
        environments,
    );
    if let Err(error) = state.workflow_runs.create(run) {
        let _ = state
            .sessions
            .rollback_turn(&desk.session, &key, &started.job.id());
        return Err(crate::error::AppError::new("store workflow run", error));
    }
    if let Some(selection) = configured_selection {
        state
            .sessions
            .set_preferred_workflow(&desk.session, key, selection.workflow_id);
    }
    state.sessions.remember_conversation(&desk.session, key);
    started.job.set_workflow_name(workflow_name);
    started.job.set_step_label("Source capture".to_owned());
    tokio::spawn(workflows::execute_run(
        state.clone(),
        WorkflowJob {
            run_id,
            session_id: desk.session,
            project_id: project.id,
            agent_id: record.id,
            agent_revision: record.revision,
            grant_alias: grant.alias,
            grant_access: grant.access,
            connection,
            host_policy: policy,
            turns: started.turns.clone(),
            job: started.job.clone(),
            eligible_reply: std::sync::Arc::new(std::sync::Mutex::new(String::new())),
        },
        lease,
        execution,
    ));

    match graft {
        CommandGraft::Document => {
            let Some(latest) = state.sessions.snapshot(&desk.session, &key) else {
                return Ok(responses::graft_redirect(graft, "/connect"));
            };
            render_document(
                &state,
                PatchStatus::Ok,
                view(
                    &state,
                    DeskPage {
                        project: &project,
                        agent: &record,
                        eligible: &eligible,
                        snapshot: &latest,
                    },
                    "",
                    "",
                    "",
                )
                .await,
            )
        }
        CommandGraft::Patch => accept_job_patch(
            &started.turns,
            &started.job.id().as_hex(),
            &desk_path(&project.id, &record.id),
            &started.job.run_id().as_hex(),
            &started.job.step_label(),
            &started.job.workflow_name(),
        ),
    }
}

pub(super) async fn cancel(
    State(state): State<AppState>,
    session: crate::sessions::RequiredSession,
    graft: CommandGraft,
    Path((project_id, agent_id, job_id)): Path<(String, String, String)>,
) -> AppResult<Response> {
    let desk = match require_desk(&state, session.0, &project_id, &agent_id, graft).await {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    if let Some(id) = sessions::JobId::parse(&job_id)
        && let Some(job) = state.sessions.job(&desk.session, &desk.key, &id)
    {
        job.request_cancel();
    }
    let Some(latest) = state.sessions.snapshot(&desk.session, &desk.key) else {
        return Ok(responses::graft_redirect(graft, "/connect"));
    };
    match graft {
        CommandGraft::Document => render_document(
            &state,
            PatchStatus::Ok,
            view(
                &state,
                DeskPage {
                    project: &desk.project,
                    agent: &desk.agent,
                    eligible: &desk.eligible,
                    snapshot: &latest,
                },
                "",
                "",
                "",
            )
            .await,
        ),
        CommandGraft::Patch => Ok(hypergraft::outcome::children_patch(
            PatchStatus::Ok,
            "job-observe",
            &view(
                &state,
                DeskPage {
                    project: &desk.project,
                    agent: &desk.agent,
                    eligible: &desk.eligible,
                    snapshot: &latest,
                },
                "",
                "",
                "",
            )
            .await
            .job_observe(),
        )?),
    }
}

async fn require_desk(
    state: &AppState,
    session: SessionId,
    project_id: &str,
    agent_id: &str,
    graft: impl Into<GraftRequest>,
) -> Result<DeskContext, Response> {
    let graft = graft.into();
    if !state.vault.has_providers() {
        return Err(responses::graft_redirect(graft, "/connect"));
    }
    let Some(project_id) = ProjectId::parse(project_id) else {
        return Err(responses::graft_redirect(graft, "/projects"));
    };
    let Some(agent_id) = crate::agents::AgentId::parse(agent_id) else {
        return Err(responses::graft_redirect(graft, "/projects"));
    };
    let Some(project) = state.projects.get(&project_id) else {
        return Err(responses::graft_redirect(graft, "/projects"));
    };
    let Some(agent) = state.agents.get(&agent_id) else {
        return Err(responses::graft_redirect(
            graft,
            &format!("/projects/{}", project.id.as_hex()),
        ));
    };
    let Some(grant) = eligibility(&agent, &project) else {
        return Err(responses::graft_redirect(
            graft,
            &format!("/projects/{}", project.id.as_hex()),
        ));
    };
    let key = ConversationKey {
        project_id: project.id,
        agent_id: agent.id,
    };
    let Some(snapshot) = state.sessions.snapshot(&session, &key) else {
        return Err(responses::graft_redirect(graft, "/connect"));
    };
    let eligible = eligible_agents(&state.agents.list(), &project);
    Ok(DeskContext {
        session,
        project,
        agent,
        grant,
        snapshot,
        key,
        eligible,
    })
}
