mod directories;
mod job;
mod new;
mod plans;
pub(crate) mod recent;
mod title;
mod tool_approval;
pub(super) use title::live_router;
mod page;
pub(crate) mod settings;
mod task_import;
mod workflow;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hypergraft::{GraftRequest, PatchGraft, PatchStatus};
use serde::Deserialize;

use crate::{
    agents::AgentId,
    conversations::{
        CandidateReviewCreation, CandidateReviewLink, ConversationError, ConversationId,
        ConversationModelConfiguration, ConversationRecord, DocumentError, DocumentId,
        PlanDocument, PlanReviewCreation, PlanReviewLink, PlanRevisionReference, PlanSource,
    },
    environments::EnvironmentId,
    error::{AppError, AppResult},
    projects::ProjectId,
    providers::{ModelSelection, ProviderKind, ThinkingEffort},
    responses,
    sessions::{JobId, RequiredSession},
    state::AppState,
    workflows::{self, WorkflowJob, WorkflowRun},
};

use self::page::{
    CandidateReviewLinkView, CandidateReviewView, CatalogueView, ConversationDetailView,
    ConversationLinkView, ModelSources, PlanDocumentPage, PlanReviewView, PresetOption,
    ProviderOption, ReviewProjectOption,
};

const REVISION_MESSAGE: &str = "Reload the conversation and try again.";

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/conversations", get(catalogue).post(create))
        .route("/conversations/new", get(new::show).post(new::save))
        .route("/conversations/new/model", post(new::remember_model))
        .route(
            "/conversations/new/settings/presets/save",
            post(settings::save_draft_preset),
        )
        .route(
            "/conversations/new/settings/presets/preview",
            post(settings::preview_draft_preset),
        )
        .route(
            "/conversations/new/settings/presets/apply",
            post(settings::apply_draft_preset),
        )
        .route(
            "/conversations/new/directories/pick",
            post(directories::pick_new),
        )
        .route(
            "/conversations/new/directories/consent",
            post(directories::approve_new),
        )
        .route(
            "/conversations/new/directories/{grant_id}/consent",
            post(directories::request_new),
        )
        .route(
            "/conversations/new/directories/{grant_id}/access",
            post(directories::update_new),
        )
        .route(
            "/conversations/new/directories/{grant_id}/remove",
            post(directories::remove_new),
        )
        .route("/conversations/{conversation_id}", get(detail))
        .route(
            "/conversations/{conversation_id}/workflow",
            get(workflow::show).post(workflow::launch),
        )
        .route(
            "/conversations/{conversation_id}/plans/from-message",
            post(request_plan_from_message),
        )
        .route(
            "/conversations/{conversation_id}/plans/request",
            post(plans::request),
        )
        .route(
            "/conversations/{conversation_id}/plans/text",
            post(save_plan_text),
        )
        .route(
            "/conversations/{conversation_id}/tasks",
            post(save_task_list_message),
        )
        .route(
            "/conversations/{conversation_id}/plans/{document_id}/tasks",
            post(prepare_tasks),
        )
        .route(
            "/conversations/{conversation_id}/tasks/import",
            post(task_import::import),
        )
        .route(
            "/conversations/{conversation_id}/tasks/text",
            post(save_task_list_text),
        )
        .route(
            "/conversations/{conversation_id}/plans/{document_id}/remove",
            post(remove_plan),
        )
        .route(
            "/conversations/{conversation_id}/plans/{document_id}/review",
            get(plan_review).post(create_plan_review),
        )
        .route(
            "/conversations/candidate-review",
            get(candidate_review).post(create_candidate_review),
        )
        .route(
            "/conversations/{conversation_id}/messages",
            post(send_message),
        )
        .route(
            "/conversations/{conversation_id}/cancel",
            post(cancel_message),
        )
        .route(
            "/conversations/{conversation_id}/runs/{run_id}/settle-partial",
            post(settle_partial),
        )
        .route(
            "/conversations/{conversation_id}/host-command/approve",
            post(tool_approval::approve),
        )
        .route(
            "/conversations/{conversation_id}/host-command/reject",
            post(tool_approval::reject),
        )
        .route(
            "/conversations/{conversation_id}/settings",
            post(settings::update),
        )
        .route(
            "/conversations/{conversation_id}/settings/host-consent",
            post(settings::request_host),
        )
        .route(
            "/conversations/{conversation_id}/settings/host-consent/approve",
            post(settings::approve_host),
        )
        .route(
            "/conversations/new/settings/host-consent",
            post(settings::request_host_draft),
        )
        .route(
            "/conversations/new/settings/host-consent/approve",
            post(settings::approve_host_draft),
        )
        .route(
            "/conversations/{conversation_id}/settings/environment",
            post(settings::preview_environment_switch),
        )
        .route(
            "/conversations/{conversation_id}/settings/presets/save",
            post(settings::save_preset),
        )
        .route(
            "/conversations/{conversation_id}/settings/presets/preview",
            post(settings::preview_preset),
        )
        .route(
            "/conversations/{conversation_id}/settings/presets/apply",
            post(settings::apply_preset),
        )
        .route(
            "/conversations/{conversation_id}/settings/environment/stop-and-switch",
            post(settings::stop_and_switch_environment),
        )
        .route(
            "/conversations/{conversation_id}/directories/pick",
            post(directories::pick_saved),
        )
        .route(
            "/conversations/{conversation_id}/directories/{grant_id}/consent",
            post(directories::request_saved),
        )
        .route(
            "/conversations/{conversation_id}/directories/consent",
            post(directories::approve_saved),
        )
        .route(
            "/conversations/{conversation_id}/directories/{grant_id}/access",
            post(directories::update_saved),
        )
        .route(
            "/conversations/{conversation_id}/directories/{grant_id}/remove",
            post(directories::remove_saved),
        )
        .route("/conversations/{conversation_id}/model", post(select_model))
        .route(
            "/conversations/{conversation_id}/projects",
            post(attach_project),
        )
        .route(
            "/conversations/{conversation_id}/projects/{project_id}",
            post(detach_project),
        )
        .route(
            "/conversations/{conversation_id}/access",
            post(grant_access),
        )
        .route(
            "/conversations/{conversation_id}/target",
            post(select_target),
        )
        .route(
            "/conversations/{conversation_id}/network",
            post(set_network),
        )
        .route(
            "/conversations/{conversation_id}/rename",
            post(rename_conversation),
        )
        .route(
            "/conversations/{conversation_id}/delete",
            post(delete_conversation),
        )
        .route("/plans/{document_id}", get(open_plan))
        .route("/plans/{document_id}/export", get(export_plan))
        .route("/plans/{document_id}/revisions", post(revise_plan))
}

#[derive(Deserialize)]
struct ConversationForm {
    #[serde(default)]
    project: String,
}

#[derive(Deserialize)]
struct RenameForm {
    title: String,
    revision: String,
}

#[derive(Deserialize)]
struct RevisionForm {
    revision: String,
}

#[derive(Deserialize)]
struct MessageForm {
    revision: String,
    message: String,
}

#[derive(Deserialize)]
struct PlanMessageForm {
    revision: String,
    message_index: String,
    title: String,
    #[serde(default)]
    request: String,
}

#[derive(Deserialize)]
struct PlanTextForm {
    revision: String,
    title: String,
    markdown: String,
}

#[derive(Deserialize)]
struct PlanRevisionForm {
    revision: String,
    title: String,
    markdown: String,
}

#[derive(Deserialize)]
struct TaskListTextForm {
    revision: String,
    title: String,
    markdown: String,
}

#[derive(Deserialize)]
struct PlanAssociationForm {
    revision: String,
    document_revision: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PlanReviewQuery {
    revision: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CandidateReviewQuery {
    run: String,
    candidate: String,
    diff_base: String,
}

#[derive(Deserialize)]
struct CandidateReviewForm {
    run: String,
    candidate: String,
    diff_base: String,
    brief: String,
    provider: String,
    model: String,
    thinking: String,
    #[serde(default)]
    preset: String,
}

#[derive(Deserialize)]
struct PlanReviewForm {
    source_revision: String,
    document_revision: String,
    brief: String,
    provider: String,
    model: String,
    thinking: String,
    #[serde(default)]
    preset: String,
    #[serde(default)]
    read_only_project: Vec<String>,
}

#[derive(Deserialize)]
struct ModelForm {
    revision: String,
    provider: String,
    model: String,
    #[serde(default)]
    thinking: String,
}

#[derive(Deserialize)]
struct ProjectForm {
    revision: String,
    project: String,
}

#[derive(Deserialize)]
struct AccessForm {
    revision: String,
    project: String,
    #[serde(default)]
    access: String,
}

#[derive(Deserialize)]
struct NetworkForm {
    revision: String,
    network: String,
    #[serde(default)]
    network_domains: String,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CatalogueQuery {
    directory: String,
    q: String,
    index: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ObserveQuery {
    job: String,
    cursor: String,
    #[serde(default)]
    title: bool,
    plans: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PlanQuery {
    revision: String,
}

async fn catalogue(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Query(query): Query<CatalogueQuery>,
) -> AppResult<Response> {
    if query.index && graft == GraftRequest::Patch {
        return recent::response(&state);
    }
    let trimmed = query.q.trim();
    let valid_directory = query.directory.is_empty()
        || (query.directory.len() == 33
            && state
                .conversations
                .list()
                .iter()
                .flat_map(page::history_grants)
                .any(|grant| page::history_directory_key(grant) == query.directory));
    let error = if !valid_directory {
        "Choose a directory from conversation history."
    } else if trimmed.len() > 256 {
        "Search is too long. Use at most 256 characters."
    } else {
        ""
    };
    render_catalogue(&state, graft, &query.directory, trimmed, error)
}

async fn create(
    State(state): State<AppState>,
    _session: RequiredSession,
    _graft: PatchGraft,
    Form(form): Form<ConversationForm>,
) -> AppResult<Response> {
    let project = if form.project.trim().is_empty() {
        None
    } else {
        let Some(project) =
            ProjectId::parse(form.project.trim()).and_then(|id| state.projects.get(&id))
        else {
            return creation_error(&state, "Choose an available project.");
        };
        Some(project)
    };
    let path = project.map_or_else(
        || "/conversations/new".to_owned(),
        |project| format!("/conversations/new?project={}", project.id.as_hex()),
    );
    Ok(responses::command_navigation(&path))
}

async fn detail(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: GraftRequest,
    Path(conversation_id): Path<String>,
    Query(query): Query<ObserveQuery>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::request_navigation(graft, "/conversations"));
    };
    if graft == GraftRequest::Patch && query.title {
        return title::response(&record);
    }
    if graft == GraftRequest::Patch && !query.job.is_empty() {
        return observe_message(state, session.0, record, query);
    }
    let mut view = detail_view(&state, session.0, &record, &record.title, "");
    view.documents_open = query.plans;
    render_detail(&state, session.0, graft, PatchStatus::Ok, view)
}

pub(super) fn refresh_after_loop_command(
    state: &AppState,
    session: crate::sessions::SessionId,
    id: &crate::conversations::ConversationId,
) -> AppResult<Response> {
    let Some(record) = state.conversations.get(id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    render_detail(
        state,
        session,
        GraftRequest::Patch,
        PatchStatus::Ok,
        detail_view(state, session, &record, &record.title, ""),
    )
}

async fn plan_review(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Path((conversation_id, document_id)): Path<(String, String)>,
    Query(query): Query<PlanReviewQuery>,
) -> AppResult<Response> {
    let Some(source) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::request_navigation(graft, "/conversations"));
    };
    let Some(document_id) = DocumentId::parse(&document_id) else {
        return Ok(responses::request_navigation(
            graft,
            &conversation_path(&source),
        ));
    };
    let Some(document) = state.documents.get(&document_id) else {
        return Ok(responses::request_navigation(
            graft,
            &conversation_path(&source),
        ));
    };
    let revision = if query.revision.is_empty() {
        document.current_revision()
    } else {
        let Some(revision) = parse_revision(&query.revision) else {
            return render_plan_review(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                &source,
                &document,
                document.current_revision(),
                "",
                "Choose an available plan revision.",
                None,
                &[],
            );
        };
        revision
    };
    if document.revision(revision).is_none() {
        return render_plan_review(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            &source,
            &document,
            document.current_revision(),
            "",
            "Choose an available plan revision.",
            None,
            &[],
        );
    }
    if !plan_origin_matches(&document, revision, source.id) {
        return Ok(responses::request_navigation(
            graft,
            &conversation_path(&source),
        ));
    }
    render_plan_review(
        &state,
        graft,
        PatchStatus::Ok,
        &source,
        &document,
        revision,
        default_review_brief(),
        "",
        None,
        &[],
    )
}

async fn create_plan_review(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path((conversation_id, document_id)): Path<(String, String)>,
    Form(fields): Form<Vec<(String, String)>>,
) -> AppResult<Response> {
    let mut projects = Vec::new();
    let fields = fields.into_iter().filter(|(key, value)| {
        if key == "read_only_project" {
            projects.push(value.clone());
            false
        } else {
            true
        }
    });
    let Ok(mut form) = PlanReviewForm::deserialize(serde::de::value::MapDeserializer::<
        _,
        serde::de::value::Error,
    >::new(fields)) else {
        return Ok(axum::http::StatusCode::UNPROCESSABLE_ENTITY.into_response());
    };
    form.read_only_project = projects;
    let Some(source) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(document_id) = DocumentId::parse(&document_id) else {
        return Ok(responses::command_navigation(&conversation_path(&source)));
    };
    let Some(document) = state.documents.get(&document_id) else {
        return Ok(responses::command_navigation(&conversation_path(&source)));
    };
    if parse_revision(&form.source_revision).is_none() {
        return render_plan_review(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            &source,
            &document,
            document.current_revision(),
            &form.brief,
            REVISION_MESSAGE,
            Some(&form),
            &[],
        );
    }
    let Some(document_revision) = parse_revision(&form.document_revision) else {
        return render_plan_review(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            &source,
            &document,
            document.current_revision(),
            &form.brief,
            "Choose an available plan revision.",
            None,
            &[],
        );
    };
    let Some(selected_revision) = document.revision(document_revision) else {
        return render_plan_review(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            &source,
            &document,
            document.current_revision(),
            &form.brief,
            "Choose an available plan revision.",
            None,
            &[],
        );
    };
    if !plan_origin_matches(&document, document_revision, source.id) {
        return render_plan_review(
            &state,
            graft,
            PatchStatus::Conflict,
            &source,
            &document,
            document_revision,
            &form.brief,
            "This plan does not belong to the source conversation.",
            None,
            &[],
        );
    }
    let model = match review_model(&state, &form) {
        Ok(model) => model,
        Err(error) => {
            return render_plan_review(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                &source,
                &document,
                document_revision,
                &form.brief,
                error,
                Some(&form),
                &[],
            );
        }
    };
    let read_only_projects = match review_projects(&state, &source, &form.read_only_project) {
        Ok(projects) => projects,
        Err(error) => {
            return render_plan_review(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                &source,
                &document,
                document_revision,
                &form.brief,
                error,
                Some(&form),
                &[],
            );
        }
    };
    let title = review_title(&document.title);
    let plan = PlanRevisionReference {
        document_id,
        revision: selected_revision.revision,
        content_hash: selected_revision.content_hash,
        object_hash: selected_revision.object_hash,
        artefact_hash: selected_revision.artefact_hash,
    };
    let review = match state.conversations.create_plan_review(PlanReviewCreation {
        source_id: source.id,
        source_revision: parse_revision(&form.source_revision).expect("validated source revision"),
        title,
        model: model.clone(),
        plan,
        task_brief: form.brief.clone(),
        read_only_projects,
        source_target: source.execution_target,
    }) {
        Ok(review) => review,
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            return Err(AppError::new("store plan review conversation", error));
        }
        Err(error) => {
            return render_plan_review(
                &state,
                graft,
                status_for(error),
                &source,
                &document,
                document_revision,
                &form.brief,
                error.message(),
                Some(&form),
                &[],
            );
        }
    };
    match start_message(
        &state,
        session.0,
        review.clone(),
        review.revision,
        model,
        form.brief,
    )
    .await
    {
        Ok(_) => Ok(responses::command_navigation(&conversation_path(&review))),
        Err(StartMessageError::Internal(error)) => Err(error),
        Err(StartMessageError::User(status, error)) => {
            let review = state.conversations.get(&review.id).unwrap_or(review);
            let view = detail_view(&state, session.0, &review, &review.title, error);
            let mut patches = hypergraft::PatchSet::new()
                .title(&view.document_title)
                .with_children("chat-main", &view)?;
            patches.replace_location(conversation_path(&review))?;
            Ok(patches.respond(status)?)
        }
    }
}

struct CandidateReviewSelection {
    run: WorkflowRun,
    candidate: crate::workflows::artefacts::ArtefactReference,
    diff_base: crate::workflows::artefacts::ArtefactReference,
    candidate_hash: crate::workflows::artefacts::CandidateHash,
    diff_base_hash: crate::workflows::artefacts::CandidateHash,
    preview: String,
}

async fn candidate_review(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Query(query): Query<CandidateReviewQuery>,
) -> AppResult<Response> {
    let selection =
        match resolve_candidate_review(&state, &query.run, &query.candidate, &query.diff_base) {
            Ok(selection) => selection,
            Err(error) => return render_candidate_review_error(&state, graft, error),
        };
    render_candidate_review(&state, graft, PatchStatus::Ok, &selection, "", None, None)
}

async fn create_candidate_review(
    State(state): State<AppState>,
    RequiredSession(session): RequiredSession,
    graft: PatchGraft,
    Form(form): Form<CandidateReviewForm>,
) -> AppResult<Response> {
    let selection =
        match resolve_candidate_review(&state, &form.run, &form.candidate, &form.diff_base) {
            Ok(selection) => selection,
            Err(error) => return render_candidate_review_error(&state, graft, error),
        };
    let source = selection
        .run
        .conversation_id
        .and_then(|id| state.conversations.get(&id));
    let model = match candidate_review_model(&state, source.as_ref(), &form) {
        Ok(model) => model,
        Err(error) => {
            return render_candidate_review(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                &selection,
                &form.brief,
                Some(error),
                Some(&form),
            );
        }
    };
    let Some(connection) = state.vault.connection_for(&model.settings.model) else {
        return render_candidate_review(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            &selection,
            &form.brief,
            Some("Choose a stored provider."),
            Some(&form),
        );
    };
    let secret = match connection.auth {
        crate::providers::AuthMethod::ApiKey => Some(connection.api_key.expose()),
        crate::providers::AuthMethod::Plan => None,
    };
    if let Err(error) = job::validate_candidate_review(
        &state,
        &selection.run,
        &selection.candidate,
        &selection.diff_base,
        secret,
    ) {
        return render_candidate_review(
            &state,
            graft,
            PatchStatus::Conflict,
            &selection,
            &form.brief,
            Some(error),
            Some(&form),
        );
    }
    let source_at_safe_gate = source_at_safe_gate(
        &state,
        &selection.run,
        &selection.candidate,
        &selection.diff_base,
        &session,
    );
    if source
        .as_ref()
        .is_some_and(|record| record.active_job.is_some())
        && !source_at_safe_gate
    {
        return render_candidate_review(
            &state,
            graft,
            PatchStatus::Conflict,
            &selection,
            &form.brief,
            Some("The source run is active. Start this review after it reaches a safe gate."),
            Some(&form),
        );
    }
    let review = match state
        .conversations
        .create_candidate_review(CandidateReviewCreation {
            source_conversation: source.as_ref().map(|record| (record.id, record.revision)),
            title: candidate_review_title(&selection.run),
            model: model.clone(),
            run_id: selection.run.id,
            candidate: selection.candidate.clone(),
            diff_base: selection.diff_base.clone(),
            task_brief: form.brief.clone(),
            source_at_safe_gate,
        }) {
        Ok(review) => review,
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            return Err(AppError::new("store candidate review conversation", error));
        }
        Err(error) => {
            return render_candidate_review(
                &state,
                graft,
                status_for(error),
                &selection,
                &form.brief,
                Some(error.message()),
                Some(&form),
            );
        }
    };
    match start_message(
        &state,
        session,
        review.clone(),
        review.revision,
        model,
        form.brief,
    )
    .await
    {
        Ok(_) => Ok(responses::command_navigation(&conversation_path(&review))),
        Err(StartMessageError::Internal(error)) => Err(error),
        Err(StartMessageError::User(status, error)) => {
            let review = state.conversations.get(&review.id).unwrap_or(review);
            render_detail(
                &state,
                session,
                graft.into(),
                status,
                detail_view(&state, session, &review, &review.title, error),
            )
        }
    }
}

fn resolve_candidate_review(
    state: &AppState,
    raw_run: &str,
    raw_candidate: &str,
    raw_diff_base: &str,
) -> Result<CandidateReviewSelection, &'static str> {
    let run_id = workflows::RunId::parse(raw_run).ok_or("Choose an available source run.")?;
    let candidate_id = workflows::ArtefactId::parse(raw_candidate)
        .ok_or("Choose an available candidate artefact.")?;
    let diff_base_id = workflows::ArtefactId::parse(raw_diff_base)
        .ok_or("Choose an available diff base artefact.")?;
    let run = state
        .workflow_runs
        .get(&run_id)
        .ok_or("The source run is unavailable.")?;
    let candidate = run
        .artefact(&candidate_id)
        .filter(|record| record.kind == workflows::definition::ArtefactKind::CandidateRevision)
        .ok_or("The selected candidate is unavailable.")?;
    let diff_base = run
        .artefact(&diff_base_id)
        .filter(|record| record.kind == workflows::definition::ArtefactKind::CandidateRevision)
        .ok_or("The selected diff base is unavailable.")?;
    let candidate_reference = crate::workflows::artefacts::ArtefactReference {
        id: candidate.id,
        kind: candidate.kind,
        artefact_hash: candidate.artefact_hash,
    };
    let diff_base_reference = crate::workflows::artefacts::ArtefactReference {
        id: diff_base.id,
        kind: diff_base.kind,
        artefact_hash: diff_base.artefact_hash,
    };
    let diff = crate::workflows::artefacts::CandidateDiff::load(
        &run,
        &diff_base_reference,
        &candidate_reference,
        &state.workflow_artefacts,
    )
    .map_err(|_| "The selected immutable candidate or diff base is unavailable.")?;
    let preview = candidate_review_preview(&diff, &state.workflow_artefacts)?;
    Ok(CandidateReviewSelection {
        run,
        candidate: candidate_reference,
        diff_base: diff_base_reference,
        candidate_hash: diff.target,
        diff_base_hash: diff.base,
        preview,
    })
}

fn candidate_review_preview(
    diff: &crate::workflows::artefacts::CandidateDiff,
    store: &crate::workflows::artefacts::WorkflowArtefactRepository,
) -> Result<String, &'static str> {
    const MAXIMUM_REVIEW_PREVIEW_BYTES: usize = 256 * 1024;
    let (total, _) = diff
        .manifest_page(0, 0)
        .map_err(|_| "The selected candidate diff is unavailable.")?;
    if total > crate::workflows::artefacts::candidate::MAXIMUM_PREVIEW_PATHS {
        return Err(
            "The selected candidate diff preview is too large. Choose a smaller candidate.",
        );
    }
    let mut preview = String::from("Changed paths:\n");
    for index in 0..total {
        let change = diff
            .change(index, store)
            .map_err(|_| "The selected candidate diff is unavailable.")?;
        preview.push_str("- ");
        preview.push_str(change.status);
        preview.push(' ');
        if !change.directory.is_empty() {
            preview.push_str(&change.directory);
            preview.push('/');
        }
        preview.push_str(&change.path);
        preview.push('\n');
        for (side, facts) in [("Before", &change.old), ("After", &change.new)] {
            if let Some(facts) = facts {
                use std::fmt::Write;
                let _ = writeln!(
                    preview,
                    "{side}: {}, executable: {}, {}",
                    facts.kind, facts.executable, facts.detail,
                );
            }
        }
        if let Some(text) = change.text {
            for fragment in text {
                preview.push_str(&fragment.text);
            }
        } else if change.binary {
            preview.push_str("Binary content is not shown.\n");
        } else if change.text_too_large {
            return Err(
                "The selected candidate diff preview is too large. Choose a smaller candidate.",
            );
        }
        if preview.len() > MAXIMUM_REVIEW_PREVIEW_BYTES {
            return Err(
                "The selected candidate diff preview is too large. Choose a smaller candidate.",
            );
        }
    }
    if crate::markdown::escape_plain(&preview).len() > MAXIMUM_REVIEW_PREVIEW_BYTES {
        return Err(
            "The selected candidate diff preview is too large. Choose a smaller candidate.",
        );
    }
    Ok(preview)
}

fn source_at_safe_gate(
    state: &AppState,
    run: &WorkflowRun,
    candidate: &crate::workflows::artefacts::ArtefactReference,
    diff_base: &crate::workflows::artefacts::ArtefactReference,
    session: &crate::sessions::SessionId,
) -> bool {
    let Some(gate) = run
        .gates
        .iter()
        .rev()
        .find(|gate| gate.state == crate::workflows::gates::HumanGateState::AwaitingDecision)
    else {
        return false;
    };
    matches!(run.state, workflows::run::RunState::AwaitingHuman { .. })
        && gate.candidate == *candidate
        && gate.diff_base == *diff_base
        && state.gate_continuations.available(&run.id, session)
}

fn candidate_review_title(run: &WorkflowRun) -> String {
    let prefix = "Review candidate: ";
    let remaining = crate::conversations::MAXIMUM_TITLE_BYTES - prefix.len();
    let source = run.pinned.definition.name();
    let end = source.floor_char_boundary(remaining.min(source.len()));
    format!("{prefix}{}", &source[..end])
}

fn candidate_review_model(
    state: &AppState,
    source: Option<&ConversationRecord>,
    form: &CandidateReviewForm,
) -> Result<ConversationModelConfiguration, &'static str> {
    let selection = if form.preset.trim().is_empty() {
        candidate_submitted_selection(state, form)?
    } else {
        let preset = AgentId::parse(form.preset.trim())
            .and_then(|id| state.agents.get(&id))
            .ok_or("Choose an available reviewer preset.")?;
        preset
            .selection
            .clone()
            .or_else(|| candidate_submitted_selection(state, form).ok())
            .or_else(|| {
                source.and_then(|record| {
                    effective_model(state, record).map(|model| model.settings.model)
                })
            })
            .ok_or("Choose a model before you start this review.")?
    };
    valid_selection(state, &selection)?;
    let environment = source
        .and_then(|record| record.model.as_ref())
        .map(|model| model.settings.environment)
        .or_else(|| default_environment(state).ok())
        .ok_or("The starter environment is unavailable.")?;
    if form.preset.trim().is_empty() {
        Ok(ConversationModelConfiguration::direct(
            selection,
            environment,
        ))
    } else {
        let preset = AgentId::parse(form.preset.trim())
            .and_then(|id| state.agents.get(&id))
            .ok_or("Choose an available reviewer preset.")?;
        Ok(ConversationModelConfiguration::from_agent_snapshot(
            &preset,
            selection,
            environment,
        ))
    }
}

fn candidate_submitted_selection(
    state: &AppState,
    form: &CandidateReviewForm,
) -> Result<ModelSelection, &'static str> {
    let provider = ProviderKind::parse(form.provider.trim()).ok_or("Choose a stored provider.")?;
    let thinking = if form.thinking.trim().is_empty() {
        None
    } else {
        Some(
            ThinkingEffort::new(form.thinking.clone())
                .ok_or("Choose an available thinking effort.")?,
        )
    };
    let selection = ModelSelection::new(provider, form.model.clone(), thinking)
        .ok_or("Enter a valid model name.")?;
    valid_selection(state, &selection)?;
    Ok(selection)
}

fn candidate_review_view_model(
    state: &AppState,
    source: Option<&ConversationRecord>,
    form: Option<&CandidateReviewForm>,
) -> (Vec<ProviderOption>, Vec<PresetOption>, String) {
    let selection = form
        .and_then(|form| candidate_submitted_selection(state, form).ok())
        .or_else(|| {
            source
                .and_then(|record| effective_model(state, record).map(|model| model.settings.model))
        })
        .or_else(|| {
            state
                .preferences
                .desk_providers(&state.vault)
                .into_iter()
                .find(|provider| provider.selected)
                .map(|provider| ModelSelection {
                    provider: provider.kind,
                    model: provider.model.clone(),
                    thinking: state.models_dev.effective_effort(
                        provider.kind,
                        &provider.model,
                        provider.thinking.as_ref(),
                    ),
                })
        });
    let providers = state
        .preferences
        .desk_providers(&state.vault)
        .into_iter()
        .map(|provider| ProviderOption {
            value: provider.kind.as_str(),
            label: provider.kind.label(),
            model: selection
                .as_ref()
                .filter(|item| item.provider == provider.kind)
                .map_or(provider.model, |item| item.model.clone()),
            thinking: selection
                .as_ref()
                .filter(|item| item.provider == provider.kind)
                .and_then(|item| item.thinking.as_ref())
                .map(|item| item.as_str().to_owned())
                .unwrap_or_default(),
            selected: selection
                .as_ref()
                .is_some_and(|item| item.provider == provider.kind),
        })
        .collect();
    let selected_preset = form.map(|form| form.preset.trim()).unwrap_or_default();
    let presets = state
        .agents
        .list()
        .into_iter()
        .map(|agent| PresetOption {
            id: agent.id.as_hex(),
            name: agent.name.clone(),
            description: agent.selection.as_ref().map_or_else(
                || "Keep the selected direct model".to_owned(),
                |item| format!("{} · {}", item.provider.label(), item.model),
            ),
            selected: agent.id.as_hex() == selected_preset,
        })
        .collect();
    let summary = if let Some(form) = form.filter(|form| !form.preset.trim().is_empty()) {
        state
            .agents
            .list()
            .into_iter()
            .find(|agent| agent.id.as_hex() == form.preset.trim())
            .map_or_else(
                || "Reviewer preset is unavailable".to_owned(),
                |agent| format!("Preset: {}", agent.name),
            )
    } else {
        selection.map_or_else(
            || "Choose a stored provider and model".to_owned(),
            |item| {
                format!(
                    "Direct model: {} · {}{}",
                    item.provider.label(),
                    item.model,
                    item.thinking
                        .as_ref()
                        .map(|effort| format!(" · Thinking: {}", effort.label()))
                        .unwrap_or_default()
                )
            },
        )
    };
    (providers, presets, summary)
}

fn default_candidate_review_brief() -> &'static str {
    "Review this candidate for correctness, risks, missing tests and unintended changes. Return findings and recommendations. Do not approve or apply the candidate."
}

fn render_candidate_review(
    state: &AppState,
    graft: impl Into<GraftRequest>,
    status: PatchStatus,
    selection: &CandidateReviewSelection,
    brief: &str,
    error: Option<&'static str>,
    form: Option<&CandidateReviewForm>,
) -> AppResult<Response> {
    let graft = graft.into();
    let source = selection
        .run
        .conversation_id
        .and_then(|id| state.conversations.get(&id));
    let (providers, presets, reviewer_summary) =
        candidate_review_view_model(state, source.as_ref(), form);
    let source_title = source.as_ref().map_or_else(
        || selection.run.pinned.definition.name().to_owned(),
        |record| record.title.clone(),
    );
    let view = CandidateReviewView {
        run_id: selection.run.id.as_hex(),
        source_title,
        candidate_id: selection.candidate.id.as_hex(),
        diff_base_id: selection.diff_base.id.as_hex(),
        candidate_hash: selection.candidate_hash.as_str(),
        diff_base_hash: selection.diff_base_hash.as_str(),
        preview: selection.preview.clone(),
        instructions_summary: "Automatic project instructions come from the selected candidate's root AGENTS.md. This discussion receives the diff and root instructions without filesystem tools. The current host worktree is not used.".to_owned(),
        brief: if brief.is_empty() { default_candidate_review_brief().to_owned() } else { brief.to_owned() },
        reviewer_summary,
        providers,
        presets,
        error: error.unwrap_or(""),
    };
    match graft {
        GraftRequest::Document => {
            let mut response =
                responses::chat_page_response("Review candidate | Power Plant", state, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            "Review candidate | Power Plant",
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::PatchSet::new()
            .title("Review candidate | Power Plant")
            .with_children("candidate-review-detail", &view.contents())?
            .respond(status)?),
    }
}

fn render_candidate_review_error(
    state: &AppState,
    graft: impl Into<GraftRequest>,
    message: &'static str,
) -> AppResult<Response> {
    let graft = graft.into();
    #[derive(askama::Template)]
    #[template(
        source = "<main data-section=\"conversations\" class=\"mx-auto max-w-4xl p-8\"><div role=\"alert\" class=\"alert alert-error\">{{ message }}</div><a href=\"/runs\" data-graft class=\"btn btn-ghost mt-4\">Runs</a></main>",
        ext = "html"
    )]
    struct ErrorView {
        message: &'static str,
    }
    let view = ErrorView { message };
    match graft {
        GraftRequest::Document => {
            let mut response =
                responses::chat_page_response("Review candidate | Power Plant", state, &view)?;
            responses::apply_patch_status(&mut response, PatchStatus::Conflict);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            "Review candidate | Power Plant",
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::PatchSet::new()
            .title("Review candidate | Power Plant")
            .with_children("chat-main", &view)?
            .respond(PatchStatus::Conflict)?),
    }
}

async fn request_plan_from_message(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<PlanMessageForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, _session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    if revision != record.revision {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(&state, _session.0, &record, &record.title, REVISION_MESSAGE),
        );
    }
    let Some(message_index) = form.message_index.parse::<usize>().ok() else {
        return render_detail_document_error(
            &state,
            _session.0,
            graft,
            &record,
            DocumentError::Source,
        );
    };
    if form.title.trim().is_empty()
        || form.title.len() > 120
        || form.title.chars().any(char::is_control)
    {
        return render_detail_document_error(
            &state,
            _session.0,
            graft,
            &record,
            DocumentError::Title,
        );
    }
    if form.request.len() > 8192 {
        return render_detail_document_error(
            &state,
            _session.0,
            graft,
            &record,
            DocumentError::Content,
        );
    }
    let text = record
        .messages
        .get(message_index)
        .map_or("", |message| message.text.as_str());
    let secret = plan_secret(&state, &[&form.title, &form.request, text]);
    if record.messages.get(message_index).is_none_or(|message| {
        message.role != crate::conversations::MessageRole::Assistant
            || message.status != crate::conversations::MessageStatus::Complete
            || message.text.trim().is_empty()
    }) {
        return render_detail_document_error(
            &state,
            _session.0,
            graft,
            &record,
            DocumentError::Source,
        );
    }
    if secret.is_some() {
        return render_detail_document_error(
            &state,
            _session.0,
            graft,
            &record,
            DocumentError::Credential,
        );
    }
    let prompt = format!(
        "Create an explicit plan with create_plan from assistant message {}. Suggested title: {}\nRequest: {}",
        message_index + 1,
        form.title,
        form.request
    );
    send_preparation(
        state,
        _session,
        graft,
        record,
        prompt,
        plans::Scope::FromMessage(message_index),
    )
    .await
}

async fn save_plan_text(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<PlanTextForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let secret = plan_secret(&state, &[&form.title, &form.markdown]);
    let error_view = |error| {
        let mut view = detail_view(&state, _session.0, &record, &record.title, error);
        if let page::ConversationPageState::Saved(saved) = &mut view.state {
            saved.plan_title = crate::tools::redact(&form.title, secret.as_deref());
            saved.plan_text = crate::tools::redact(&form.markdown, secret.as_deref());
            saved.plan_text_error = true;
        }
        view
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            error_view(REVISION_MESSAGE),
        );
    };
    if revision != record.revision {
        return render_detail_command(graft, PatchStatus::Conflict, error_view(REVISION_MESSAGE));
    }
    if record.active_job.is_some() {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            error_view(DocumentError::Active.message()),
        );
    }
    match state.documents.publish_action(
        &record,
        crate::conversations::DocumentAction {
            title: form.title.clone(),
            markdown: form.markdown.clone(),
            assistant: false,
            message_index: record.messages.len().saturating_sub(1),
            previous: None,
            plan: None,
        },
        secret.as_deref(),
    ) {
        Ok(_) => {
            let view = detail_view(&state, _session.0, &record, &record.title, "");
            let mut patches = hypergraft::PatchSet::new()
                .title(&view.document_title)
                .with_children("conversation-detail", &view.contents())?;
            patches.replace_location(conversation_path(&record))?;
            Ok(patches.respond(PatchStatus::Ok)?)
        }
        Err(error @ (DocumentError::Persist | DocumentError::Corrupt)) => {
            Err(AppError::new("store plan document", error))
        }
        Err(error) => {
            render_detail_command(graft, document_status(error), error_view(error.message()))
        }
    }
}

async fn prepare_tasks(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path((conversation_id, document_id)): Path<(String, String)>,
    Form(form): Form<PlanAssociationForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let document = DocumentId::parse(&document_id).and_then(|id| state.documents.get(&id));
    let Some(document) = document.filter(|document| {
        document.associated_conversation == Some(record.id)
            && document.kind == crate::conversations::DocumentKind::Plan
            && parse_revision(&form.document_revision)
                .and_then(|revision| document.revision(revision))
                .is_some()
    }) else {
        return render_preparation_document_error(
            &state,
            session.0,
            graft,
            &record,
            DocumentError::Conflict,
        );
    };
    if parse_revision(&form.revision) != Some(record.revision) {
        return render_preparation_document_error(
            &state,
            session.0,
            graft,
            &record,
            DocumentError::Conflict,
        );
    }
    let selected = document
        .revision(parse_revision(&form.document_revision).expect("validated revision"))
        .expect("selected plan revision");
    let prompt = format!(
        "Create an explicit task breakdown with create_task_breakdown from the selected plan. Submit Markdown, without an outer code fence. Use a level-one heading, shared context preamble, and at most 128 ordered top-level '- [ ] Task' entries with indented details. Put literal checkbox examples inside code fences. Preserve the plan requirements. Do not execute tasks or modify project files.\n\nSelected plan: {}\nRevision: {}\nContent hash: {}",
        document.id,
        selected.revision,
        selected.content_hash.as_str()
    );
    send_preparation(
        state,
        session,
        graft,
        record,
        prompt,
        plans::Scope::Tasks(document.id, selected.revision),
    )
    .await
}

async fn send_preparation(
    state: AppState,
    session: RequiredSession,
    graft: PatchGraft,
    record: ConversationRecord,
    prompt: String,
    scope: plans::Scope,
) -> AppResult<Response> {
    let Some(model) = effective_model(&state, &record) else {
        return render_preparation_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                ConversationError::Selection.message(),
            ),
        );
    };
    // Preparation uses the model without guest tools, even when this conversation has write authority.
    let mut dispatch = record.clone();
    dispatch.execution_target = None;
    match start_message_mode(
        &state,
        session.0,
        dispatch,
        record.revision,
        model,
        prompt,
        MessageMode::Plan(scope),
    )
    .await
    {
        Ok(started) => render_preparation_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &started, &started.title, ""),
        ),
        Err(StartMessageError::Internal(error)) => Err(error),
        Err(StartMessageError::User(status, error)) => {
            let current = state.conversations.get(&record.id).unwrap_or(record);
            render_preparation_command(
                graft,
                status,
                detail_view(&state, session.0, &current, &current.title, error),
            )
        }
    }
}

fn render_preparation_document_error(
    state: &AppState,
    session: crate::sessions::SessionId,
    graft: PatchGraft,
    record: &ConversationRecord,
    error: DocumentError,
) -> AppResult<Response> {
    render_preparation_command(
        graft,
        PatchStatus::Conflict,
        detail_view(state, session, record, &record.title, error.message()),
    )
}

fn render_preparation_command(
    _graft: PatchGraft,
    status: PatchStatus,
    view: ConversationDetailView,
) -> AppResult<Response> {
    let id = &view.saved().expect("saved preparation conversation").id;
    Ok(hypergraft::PatchSet::new()
        .title(&view.document_title)
        .with_replace_location(format!("/conversations/{id}"))?
        .with_children("chat-main", &view)?
        .respond(status)?)
}

async fn save_task_list_message(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<PlanMessageForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, _session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    if revision != record.revision {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(&state, _session.0, &record, &record.title, REVISION_MESSAGE),
        );
    }
    let Some(message_index) = form.message_index.parse::<usize>().ok() else {
        return render_detail_document_error(
            &state,
            _session.0,
            graft,
            &record,
            DocumentError::Source,
        );
    };
    let source_text = record
        .messages
        .get(message_index)
        .map_or("", |message| message.text.as_str());
    let secret = plan_secret(&state, &[&form.title, source_text]);
    match state.documents.create_task_list_from_message(
        &record,
        message_index,
        form.title,
        secret.as_deref(),
    ) {
        Ok(_) => Ok(responses::command_navigation(&conversation_path(&record))),
        Err(error @ (DocumentError::Persist | DocumentError::Corrupt)) => {
            Err(AppError::new("store task list", error))
        }
        Err(error) => render_detail_document_error(&state, _session.0, graft, &record, error),
    }
}

async fn save_task_list_text(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<TaskListTextForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, _session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    if revision != record.revision {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(&state, _session.0, &record, &record.title, REVISION_MESSAGE),
        );
    }
    let secret = plan_secret(&state, &[&form.title, &form.markdown]);
    match state.documents.create_task_list_from_text(
        record.id,
        form.title.clone(),
        form.markdown.clone(),
        secret.as_deref(),
    ) {
        Ok(_) => Ok(responses::command_navigation(&conversation_path(&record))),
        Err(error @ (DocumentError::Persist | DocumentError::Corrupt)) => {
            Err(AppError::new("store task list", error))
        }
        Err(DocumentError::TaskList) => {
            let mut view = detail_view(
                &state,
                _session.0,
                &record,
                &record.title,
                DocumentError::TaskList.message(),
            );
            view = view.with_task_text(form.title, form.markdown);
            render_detail_command(graft, PatchStatus::UnprocessableEntity, view)
        }
        Err(error) => render_detail_document_error(&state, _session.0, graft, &record, error),
    }
}

async fn open_plan(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: GraftRequest,
    Path(document_id): Path<String>,
    Query(query): Query<PlanQuery>,
) -> AppResult<Response> {
    let Some(document_id) = DocumentId::parse(&document_id) else {
        return Ok(responses::request_navigation(graft, "/conversations"));
    };
    let Some(document) = state.documents.get(&document_id) else {
        return Ok(responses::request_navigation(graft, "/conversations"));
    };
    let revision = if query.revision.is_empty() {
        document.current_revision()
    } else {
        let Some(revision) = parse_revision(&query.revision) else {
            return render_plan_page(
                &state,
                graft,
                PatchStatus::UnprocessableEntity,
                &document,
                document.current_revision(),
                "Choose an available plan revision.",
            );
        };
        revision
    };
    if document.revision(revision).is_none() {
        return render_plan_page(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            &document,
            document.current_revision(),
            "Choose an available plan revision.",
        );
    }
    let content = state
        .documents
        .content(&document, revision)
        .map_err(|error| AppError::new("read plan document", error))?;
    if graft != GraftRequest::Patch
        && let Some(record) = document
            .associated_conversation
            .and_then(|id| state.conversations.get(&id))
    {
        let plan = PlanDocumentPage::from_document(&document, revision, content.clone(), "")
            .with_context(&state, &document);
        let html = askama::Template::render(&plan.contents())
            .map_err(|error| AppError::new("render plan companion", error))?;
        // Large plans use the standalone representation to reserve envelope space for conversation controls.
        if html.len() <= 256 * 1024 {
            let view = detail_view(&state, _session.0, &record, &record.title, "")
                .with_companion(html, "plan");
            return render_detail(&state, _session.0, graft, PatchStatus::Ok, view);
        }
    }
    render_plan_page_with_content(
        &state,
        graft,
        PatchStatus::Ok,
        &document,
        revision,
        content,
        "",
    )
}

async fn export_plan(
    State(state): State<AppState>,
    _session: RequiredSession,
    Path(document_id): Path<String>,
    Query(query): Query<PlanQuery>,
) -> AppResult<Response> {
    let Some(document_id) = DocumentId::parse(&document_id) else {
        return Ok(responses::no_store_status_response(
            axum::http::StatusCode::NOT_FOUND,
            "Plan not found",
        ));
    };
    let Some(document) = state.documents.get(&document_id) else {
        return Ok(responses::no_store_status_response(
            axum::http::StatusCode::NOT_FOUND,
            "Plan not found",
        ));
    };
    let revision = if query.revision.is_empty() {
        document.current_revision()
    } else {
        let Some(revision) = parse_revision(&query.revision) else {
            return Ok(responses::no_store_status_response(
                axum::http::StatusCode::BAD_REQUEST,
                "Plan revision is invalid",
            ));
        };
        revision
    };
    let Some(selected) = document.revision(revision) else {
        return Ok(responses::no_store_status_response(
            axum::http::StatusCode::NOT_FOUND,
            "Plan revision not found",
        ));
    };
    let content = state
        .documents
        .content(&document, revision)
        .map_err(|error| AppError::new("read plan document", error))?;
    let filename = safe_filename(&document.title);
    let disposition = format!(
        "attachment; filename=\"{filename}-r{}.md\"",
        selected.revision
    );
    let mut response = (
        axum::http::StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/markdown; charset=utf-8"),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                axum::http::HeaderValue::from_str(&disposition).unwrap_or_else(|_| {
                    axum::http::HeaderValue::from_static("attachment; filename=plan.md")
                }),
            ),
        ],
        content,
    )
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

async fn revise_plan(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PatchGraft,
    Path(document_id): Path<String>,
    Form(form): Form<PlanRevisionForm>,
) -> AppResult<Response> {
    let Some(document_id) = DocumentId::parse(&document_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(document) = state.documents.get(&document_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    if let Some(conversation_id) = document.associated_conversation
        && let Some(conversation) = state.conversations.get(&conversation_id)
        && conversation.active_job.is_some()
    {
        let revision = document.current_revision();
        return render_plan_page_with_content(
            &state,
            graft,
            PatchStatus::Conflict,
            &document,
            revision,
            state
                .documents
                .content(&document, revision)
                .map_err(|error| AppError::new("read plan document", error))?,
            DocumentError::Active.message(),
        );
    }
    let Some(revision) = parse_revision(&form.revision) else {
        return render_plan_page_with_content(
            &state,
            graft,
            PatchStatus::UnprocessableEntity,
            &document,
            document.current_revision(),
            state
                .documents
                .content(&document, document.current_revision())
                .map_err(|error| AppError::new("read plan document", error))?,
            DocumentError::Conflict.message(),
        );
    };
    let secret = plan_secret(&state, &[&form.title, &form.markdown]);
    let owner = document
        .associated_conversation
        .and_then(|id| state.conversations.get(&id));
    let result = if let Some(owner) =
        owner.filter(|_| document.kind == crate::conversations::DocumentKind::Plan)
    {
        state.documents.publish_action(
            &owner,
            crate::conversations::DocumentAction {
                title: form.title.clone(),
                markdown: form.markdown.clone(),
                assistant: false,
                message_index: owner.messages.len().saturating_sub(1),
                previous: Some((document.id, revision)),
                plan: None,
            },
            secret.as_deref(),
        )
    } else {
        state.documents.revise(
            &document.id,
            revision,
            form.title.clone(),
            form.markdown.clone(),
            secret.as_deref(),
        )
    };
    match result {
        Ok(updated) => Ok(responses::command_navigation(&format!(
            "/plans/{}",
            updated.id
        ))),
        Err(error @ (DocumentError::Persist | DocumentError::Corrupt)) => {
            Err(AppError::new("store plan revision", error))
        }
        Err(DocumentError::TaskList) => {
            let content = state
                .documents
                .content(&document, document.current_revision())
                .map_err(|error| AppError::new("read task list revision", error))?;
            let mut view = PlanDocumentPage::from_document(
                &document,
                document.current_revision(),
                content,
                DocumentError::TaskList.message(),
            );
            view.conversation_revision = document
                .associated_conversation
                .and_then(|id| state.conversations.get(&id))
                .map_or(0, |record| record.revision);
            view.content = form.markdown;
            Ok(hypergraft::PatchSet::new()
                .title(&view.document_title)
                .with_children("plan-detail", &view.contents())?
                .respond(PatchStatus::UnprocessableEntity)?)
        }
        Err(error) => {
            let latest = state.documents.get(&document.id).unwrap_or(document);
            let selected = latest.current_revision();
            let content = state
                .documents
                .content(&latest, selected)
                .map_err(|error| AppError::new("read plan document", error))?;
            render_plan_page_with_content(
                &state,
                graft,
                document_status(error),
                &latest,
                selected,
                content,
                error.message(),
            )
        }
    }
}

async fn remove_plan(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path((conversation_id, document_id)): Path<(String, String)>,
    Form(form): Form<PlanAssociationForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(document_id) = DocumentId::parse(&document_id) else {
        return render_detail_document_error(
            &state,
            session.0,
            graft,
            &record,
            DocumentError::Missing,
        );
    };
    let Some(conversation_revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(document_revision) = parse_revision(&form.document_revision) else {
        return render_detail_document_error(
            &state,
            session.0,
            graft,
            &record,
            DocumentError::Conflict,
        );
    };
    if conversation_revision != record.revision || record.active_job.is_some() {
        return render_detail_document_error(
            &state,
            session.0,
            graft,
            &record,
            if record.active_job.is_some() {
                DocumentError::Active
            } else {
                DocumentError::Conflict
            },
        );
    }
    match state
        .documents
        .disassociate(&document_id, document_revision, record.id)
    {
        Ok(()) => Ok(responses::command_navigation(&conversation_path(&record))),
        Err(error @ (DocumentError::Persist | DocumentError::Corrupt)) => {
            Err(AppError::new("remove plan association", error))
        }
        Err(error) => render_detail_document_error(&state, session.0, graft, &record, error),
    }
}

async fn send_message(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<MessageForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(model) = effective_model(&state, &record) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                ConversationError::Selection.message(),
            ),
        );
    };
    match start_message(
        &state,
        session.0,
        record.clone(),
        revision,
        model,
        form.message,
    )
    .await
    {
        Ok(started) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &started, &started.title, ""),
        ),
        Err(StartMessageError::Internal(error)) => Err(error),
        Err(StartMessageError::User(status, error)) => render_detail_command(
            graft,
            status,
            detail_view(&state, session.0, &record, &record.title, error),
        ),
    }
}

pub(super) enum StartMessageError {
    User(PatchStatus, &'static str),
    Internal(AppError),
}

pub(super) async fn preflight_execution(
    state: &AppState,
    session: crate::sessions::SessionId,
    conversation: Option<ConversationId>,
    model: &ConversationModelConfiguration,
) -> Result<(), StartMessageError> {
    if model.settings.location == crate::execution::ToolLocation::Sandbox
        && let Some(conversation) = conversation
        && model.settings.directories.iter().any(|grant| {
            (grant.access != crate::execution::DirectoryAccess::ReadOnly
                || crate::execution::authority::sensitive_directory(
                    &grant.host_path,
                    state.local_data.root(),
                ))
                && (!state.sessions.contains_live(&session)
                    || (!state.access_consent.authorised_conversation(
                        session,
                        conversation,
                        &model.settings,
                        grant,
                    ) && !state.conversations.directory_approved(
                        &conversation,
                        &model.settings,
                        grant,
                    )))
        })
    {
        return Err(StartMessageError::User(
            PatchStatus::UnprocessableEntity,
            "Directory access needs explicit approval. Open Directories to approve it.",
        ));
    }
    if model.settings.location == crate::execution::ToolLocation::Host {
        if model.settings.host_tools() {
            for grant in &model.settings.directories {
                grant.revalidate().map_err(|_| {
                    StartMessageError::User(
                        PatchStatus::UnprocessableEntity,
                        "A work location changed or is not available.",
                    )
                })?;
            }
        }
        if model.settings.host_tools()
            && let Some(conversation) = conversation
            && (!state.sessions.contains_live(&session)
                || !state.access_consent.authorised_host_conversation(
                    session,
                    conversation,
                    &model.settings,
                ))
        {
            return Err(StartMessageError::User(
                PatchStatus::UnprocessableEntity,
                "Unrestricted host access needs explicit approval. Open Settings to approve it.",
            ));
        }
        return Ok(());
    }
    if model.settings.tools.is_empty() {
        return Ok(());
    }
    crate::execution::ProjectFreeAuthority::from_settings(1, &model.settings)
        .map_err(|error| StartMessageError::User(PatchStatus::Conflict, error.message()))?;
    if let Some(missing) = state.sandboxes.missing() {
        return Err(StartMessageError::User(
            PatchStatus::UnprocessableEntity,
            missing.message(),
        ));
    }
    let environment = model.settings.environment;
    let directories = model
        .settings
        .directories
        .iter()
        .map(|grant| workflows::definition::GuestDirectoryAccess {
            alias: grant.alias.clone(),
            access: crate::agents::AccessMode::ReadOnly,
        })
        .collect();
    let pinned = workflows::pin_project_free_quick_task_with_directories(
        &model.settings.tools,
        &model.settings.instructions,
        environment,
        directories,
        model
            .settings
            .directories
            .iter()
            .any(|grant| grant.access == crate::execution::DirectoryAccess::ReviewBeforeApply),
    )
    .map_err(|error| StartMessageError::User(PatchStatus::UnprocessableEntity, error.message()))?;
    workflows::resolve_environments(
        &pinned.definition,
        &state.environments,
        &state.environment_snapshots,
    )
    .await
    .map(|_| ())
    .map_err(|error| StartMessageError::User(PatchStatus::UnprocessableEntity, error.message()))
}

pub(super) async fn start_message(
    state: &AppState,
    session: crate::sessions::SessionId,
    record: ConversationRecord,
    revision: u32,
    model: ConversationModelConfiguration,
    text: String,
) -> Result<ConversationRecord, StartMessageError> {
    start_message_mode(
        state,
        session,
        record,
        revision,
        model,
        text,
        MessageMode::Conversation,
    )
    .await
}

#[derive(Clone, Copy)]
enum MessageMode {
    Conversation,
    Plan(plans::Scope),
}

async fn start_message_mode(
    state: &AppState,
    session: crate::sessions::SessionId,
    record: ConversationRecord,
    revision: u32,
    mut model: ConversationModelConfiguration,
    text: String,
    mode: MessageMode,
) -> Result<ConversationRecord, StartMessageError> {
    let persisted_model = model.clone();
    if matches!(mode, MessageMode::Plan(_)) {
        model.settings.tools.clear();
        model.settings.directories.clear();
    }
    preflight_execution(state, session, Some(record.id), &model).await?;
    if record.active_job.is_some() {
        return Err(StartMessageError::User(
            PatchStatus::Conflict,
            ConversationError::Active.message(),
        ));
    }
    if let Err(error) = valid_selection(state, &model.settings.model) {
        return Err(StartMessageError::User(
            PatchStatus::UnprocessableEntity,
            error,
        ));
    }
    let Some(connection) = state.vault.connection_for(&model.settings.model) else {
        return Err(StartMessageError::User(
            PatchStatus::UnprocessableEntity,
            "Choose a stored provider.",
        ));
    };
    if record.candidate_review_context.is_some() && record.execution_target.is_some() {
        return Err(StartMessageError::User(
            PatchStatus::Conflict,
            "Candidate reviews use immutable evidence only. Remove the execution target before you continue this discussion.",
        ));
    }
    let authority = match crate::conversations::resolve_workflow_authority(
        &record,
        &state.projects,
        &state.agents,
    ) {
        Ok(authority) => authority.map(|authority| authority.effective),
        Err(error) => {
            return Err(StartMessageError::User(
                PatchStatus::Conflict,
                error.message(),
            ));
        }
    };
    let advertised_tools = crate::tools::advertised(&model.settings.tools, model.settings.location);
    let workflow = if model.settings.location != crate::execution::ToolLocation::Host
        && !advertised_tools.is_empty()
    {
        if authority.is_some() && !model.settings.directories.is_empty() {
            return Err(StartMessageError::User(
                PatchStatus::Conflict,
                "Remove legacy project access before tools use conversation directories.",
            ));
        }
        let project_free = authority
            .is_none()
            .then(|| crate::conversations::resolve_project_free_authority(&record, &state.agents))
            .transpose()
            .map_err(|error| StartMessageError::User(PatchStatus::Conflict, error.message()))?;
        let phase_authority = authority.clone();
        let environment = model.settings.environment;
        let pinned = if let Some(authority) = authority.as_ref() {
            let phase_authority = phase_authority.as_ref().expect("project authority");
            let secondary = phase_authority
                .policy
                .grants()
                .iter()
                .filter(|grant| grant.alias != authority.grant_alias)
                .map(|grant| workflows::definition::GuestDirectoryAccess {
                    alias: grant.alias.clone(),
                    access: crate::agents::AccessMode::ReadOnly,
                })
                .collect();
            workflows::pin_quick_task_with_context(
                phase_authority.grant_access,
                &phase_authority.tools,
                &model.settings.instructions,
                environment,
                secondary,
            )
        } else {
            let project_free = project_free
                .as_ref()
                .expect("project-free conversation authority");
            let directories = project_free
                .policy
                .grants()
                .iter()
                .map(|grant| workflows::definition::GuestDirectoryAccess {
                    alias: grant.alias.clone(),
                    access: crate::agents::AccessMode::ReadOnly,
                })
                .collect();
            workflows::pin_project_free_quick_task_with_directories(
                &project_free.tools,
                &model.settings.instructions,
                environment,
                directories,
                !project_free.reviewed_aliases.is_empty(),
            )
        }
        .map_err(|error| {
            StartMessageError::User(PatchStatus::UnprocessableEntity, error.message())
        })?;
        let environments = workflows::resolve_environments(
            &pinned.definition,
            &state.environments,
            &state.environment_snapshots,
        )
        .await
        .map_err(|error| {
            StartMessageError::User(PatchStatus::UnprocessableEntity, error.message())
        })?;
        let execution = state.workflow_execution.acquire().map_err(|_| {
            StartMessageError::User(
                PatchStatus::Conflict,
                "Wait until the current workflow finishes.",
            )
        })?;
        let run_id = workflows::RunId::generate().map_err(|error| {
            StartMessageError::Internal(AppError::new("create workflow run identifier", error))
        })?;
        Some((
            run_id,
            authority,
            project_free,
            pinned,
            environments,
            execution,
        ))
    } else {
        None
    };
    let job = state
        .sessions
        .begin_conversation_job(&session, record.id, record.messages.len() + 1)
        .map_err(|_| {
            StartMessageError::User(
                PatchStatus::Conflict,
                "Another command is active in this browser session.",
            )
        })?;
    let launch_brief = text.trim().to_owned();
    let phase_model = model.clone();
    let started = match state.conversations.begin_message_with_model(
        &record.id,
        revision,
        Some(persisted_model),
        job.id(),
        text,
    ) {
        Ok(started) => started,
        Err(error) => {
            state
                .sessions
                .finish_conversation_job(&session, record.id, job.id());
            return Err(StartMessageError::User(status_for(error), error.message()));
        }
    };
    if let Some((run_id, authority, project_free, pinned, environments, execution)) = workflow {
        let secret = match connection.auth {
            crate::providers::AuthMethod::ApiKey => Some(connection.api_key.expose()),
            crate::providers::AuthMethod::Plan => None,
        };
        let turns = match job::history_with_review(state, &started, secret) {
            Ok(turns) => turns,
            Err(error) => {
                let _ = state.conversations.settle_message(
                    &started.id,
                    job.id(),
                    String::new(),
                    crate::conversations::MessageStatus::Failed,
                    Some(error.to_owned()),
                );
                let _ = state
                    .sessions
                    .finish_conversation_job(&session, started.id, job.id());
                return Err(StartMessageError::User(
                    PatchStatus::UnprocessableEntity,
                    error,
                ));
            }
        };
        let phase_models = pinned
            .definition
            .steps()
            .iter()
            .filter(|step| matches!(&step.action, workflows::definition::StepAction::Agent(_)))
            .map(|step| workflows::PhaseModelSelection {
                step: step.key.clone(),
                selection: phase_model.settings.model.clone(),
                instructions: phase_model.settings.instructions.clone(),
                preset: phase_model
                    .preset
                    .as_ref()
                    .map(|preset| workflows::PinnedPreset {
                        id: preset.id,
                        revision: preset.revision,
                        name: preset.name.clone(),
                    }),
                settings: Some(phase_model.settings.clone()),
            })
            .collect::<Vec<_>>();
        let mut run = match authority.as_ref() {
            Some(authority) => WorkflowRun::create_for_conversation(
                run_id,
                workflows::now_ms(),
                authority.project_id,
                started.id,
                pinned,
                environments,
                phase_models,
            ),
            None => WorkflowRun::create_source_free_for_conversation(
                run_id,
                workflows::now_ms(),
                started.id,
                pinned,
                environments,
                phase_models,
            ),
        };
        run.launch_brief = launch_brief;
        if let Err(error) = state.workflow_runs.create(run.clone()) {
            let _ = state.conversations.settle_message(
                &started.id,
                job.id(),
                String::new(),
                crate::conversations::MessageStatus::Failed,
                None,
            );
            let _ = state
                .sessions
                .finish_conversation_job(&session, started.id, job.id());
            return Err(StartMessageError::Internal(AppError::new(
                "store workflow run",
                error,
            )));
        }
        job.set_workflow_name(run.pinned.definition.name().to_owned());
        job.set_step_label(if authority.is_some() {
            "Source capture".to_owned()
        } else {
            "Preparing private workspace".to_owned()
        });
        let host_policy = authority
            .as_ref()
            .map(|authority| authority.policy.clone())
            .or_else(|| {
                project_free
                    .as_ref()
                    .map(|authority| authority.policy.clone())
            })
            .expect("tool execution authority");
        tokio::spawn(workflows::execute_run(
            state.clone(),
            WorkflowJob {
                run_id,
                session_id: session,
                project_id: run.project_id,
                agent_id: run.agent_id,
                agent_revision: authority
                    .as_ref()
                    .map_or(record.revision, |authority| authority.revision),
                conversation_id: Some(started.id),
                authority: authority.clone(),
                project_free_authority: project_free,
                grant_alias: authority
                    .as_ref()
                    .map_or_else(String::new, |authority| authority.grant_alias.clone()),
                grant_access: authority
                    .as_ref()
                    .map_or(crate::agents::AccessMode::ReadWrite, |authority| {
                        authority.grant_access
                    }),
                connection,
                phase_providers: run
                    .model_phases()
                    .map(|phase| phase.selection.provider)
                    .collect(),
                active_connection: std::sync::Arc::new(std::sync::Mutex::new(None)),
                host_policy,
                turns,
                job: job.clone(),
                eligible_reply: std::sync::Arc::new(std::sync::Mutex::new(String::new())),
                task_loop: None,
            },
            None,
            execution,
        ));
    } else if !advertised_tools.is_empty() {
        tokio::spawn(job::run_host_tools(
            state.clone(),
            session,
            started.id,
            started,
            connection,
            job,
        ));
    } else {
        tokio::spawn(job::run(
            state.clone(),
            session,
            started.id,
            started,
            connection,
            job,
            match mode {
                MessageMode::Conversation => None,
                MessageMode::Plan(scope) => Some(scope),
            },
        ));
    }
    Ok(state.conversations.get(&record.id).unwrap_or(record))
}

async fn cancel_message(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<ObserveQuery>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(job_id) = JobId::parse(&form.job) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(job) = state.sessions.conversation_job(record.id, job_id) else {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "This reply is no longer active.",
            ),
        );
    };
    job.request_cancel();
    state.host_approvals.invalidate_job(job.id());
    render_detail_command(
        graft,
        PatchStatus::Ok,
        detail_view(&state, session.0, &record, &record.title, ""),
    )
}

#[derive(Deserialize)]
struct SettlePartialForm {
    #[serde(default)]
    attempt: String,
    #[serde(default)]
    state: String,
}

/// Keep applied files and end a known partial task without another write.
/// Every command revalidates the stored run; disabled controls grant nothing.
async fn settle_partial(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path((conversation_id, run_id)): Path<(String, String)>,
    Form(form): Form<SettlePartialForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(run_id) = crate::workflows::RunId::parse(&run_id) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(run) = state.workflow_runs.get(&run_id) else {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    if run.conversation_id != Some(record.id) {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    }
    // Loop children end through the task loop controls, not direct settlement.
    if run.parent_loop.is_some() {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "That task belongs to a task loop. Use the task loop controls to end it.",
            ),
        );
    }
    let outcome_matches = run
        .latest_apply_attempt()
        .is_some_and(|attempt| attempt.id.as_hex() == form.attempt);
    if !outcome_matches {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "That file application changed. Reload the conversation and try again.",
            ),
        );
    }
    // Recovery protection retains its reservations; settlement never force-unlocks.
    if state.gate_continuations.commit_recovery_locked() {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "This operation requires recovery. The conversation remains reserved until a restart reconciles the local records.",
            ),
        );
    }
    if !run.partial_settlement_eligible() {
        let message = if run.apply_is_uncertain() {
            "Execution remains unsettled. Continuation and retry stay unavailable until recovery and cleanup finish."
        } else if run.is_terminal() {
            "That task already ended. Continue the conversation for the next task."
        } else {
            "That file application changed. Reload the conversation and try again."
        };
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(&state, session.0, &record, &record.title, message),
        );
    }
    let expected_state = run
        .latest_apply_attempt()
        .and_then(|attempt| attempt.apply_transaction.as_ref())
        .map(|transaction| match transaction.state {
            crate::workflows::apply::ApplyTransactionState::Recovered => "recovered",
            _ => "",
        })
        .unwrap_or("");
    if form.state != expected_state {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "That file application changed. Reload the conversation and try again.",
            ),
        );
    }
    if state
        .workflow_runs
        .mutate(&run_id, |run| {
            run.settle_known_partial(crate::workflows::now_ms())
        })
        .is_err()
    {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "That file application changed. Reload the conversation and try again.",
            ),
        );
    }
    let record = load_conversation(&state, &conversation_id).unwrap_or(record);
    render_detail_command(
        graft,
        PatchStatus::Ok,
        detail_view(&state, session.0, &record, &record.title, ""),
    )
}

fn observe_message(
    state: AppState,
    session: crate::sessions::SessionId,
    record: ConversationRecord,
    query: ObserveQuery,
) -> AppResult<Response> {
    if let Some(job) =
        JobId::parse(&query.job).and_then(|id| state.sessions.conversation_job(record.id, id))
    {
        let cursor = query
            .cursor
            .parse::<u64>()
            .unwrap_or(0)
            .min(job.latest_seq());
        return Ok(job::observe_response(
            state, record.id, session, job, cursor,
        ));
    }
    render_detail(
        &state,
        session,
        GraftRequest::Patch,
        PatchStatus::Ok,
        detail_view(&state, session, &record, &record.title, ""),
    )
}

async fn select_model(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<ModelForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(provider) = ProviderKind::parse(form.provider.trim()) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose a stored provider.",
            ),
        );
    };
    let has_thinking = !form.thinking.trim().is_empty();
    let thinking = if has_thinking {
        ThinkingEffort::new(form.thinking)
    } else {
        None
    };
    if has_thinking && thinking.is_none() {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose an available thinking effort.",
            ),
        );
    }
    let Some(selection) = ModelSelection::new(provider, form.model, thinking) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Enter a valid model name.",
            ),
        );
    };
    if let Err(error) = valid_selection(&state, &selection) {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, error),
        );
    }
    let environment = record
        .model
        .as_ref()
        .map(|model| model.settings.environment)
        .or_else(|| default_environment(&state).ok())
        .ok_or_else(|| {
            AppError::new(
                "select conversation environment",
                std::io::Error::other("starter environment unavailable"),
            )
        })?;
    match state
        .conversations
        .select_model(&record.id, revision, selection.clone(), environment)
    {
        Ok(updated) => {
            let warning = remember_selection(&state, selection).err().unwrap_or("");
            render_detail_command(
                graft,
                PatchStatus::Ok,
                detail_view(&state, session.0, &updated, &updated.title, warning),
            )
        }
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store model selection", error))
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn attach_project(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<ProjectForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    if state.sessions.busy(&session.0) {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Another command is active in this browser session.",
            ),
        );
    }
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(project) =
        ProjectId::parse(&form.project).filter(|project| state.projects.get(project).is_some())
    else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose a project from the catalogue.",
            ),
        );
    };
    match state
        .conversations
        .attach_project(&record.id, revision, project)
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store project context", error))
        }
        Err(ConversationError::Conflict) => {
            let latest = state.conversations.get(&record.id).unwrap_or(record);
            render_detail_command(
                graft,
                PatchStatus::Conflict,
                detail_view(
                    &state,
                    session.0,
                    &latest,
                    &latest.title,
                    ConversationError::Conflict.message(),
                ),
            )
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn grant_access(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<AccessForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    if state.sessions.busy(&session.0) {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Another command is active in this browser session.",
            ),
        );
    }
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(project) = ProjectId::parse(&form.project).and_then(|id| state.projects.get(&id))
    else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose an attached project.",
            ),
        );
    };
    if !record.projects.contains(&project.id) {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Attach this project before you grant access.",
            ),
        );
    }
    if !project.host_path_is_available() {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "The selected project is unavailable.",
            ),
        );
    }
    let access = if form.access.trim().is_empty() {
        crate::agents::AccessMode::ReadOnly
    } else if let Some(access) = crate::agents::AccessMode::parse(&form.access) {
        access
    } else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose read-only or writable access.",
            ),
        );
    };
    let updated = match access {
        crate::agents::AccessMode::ReadOnly => {
            state
                .conversations
                .grant_read_only(&record.id, revision, project.id, project.revision)
        }
        crate::agents::AccessMode::ReadWrite => {
            state
                .conversations
                .grant_writable(&record.id, revision, project.id, project.revision)
        }
    };
    match updated {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store conversation access", error))
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn set_network(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<NetworkForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    if state.sessions.busy(&session.0)
        || record.active_job.is_some()
        || state.sessions.conversation_reserved(record.id)
    {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "This conversation is reserved. Wait until its operation finishes.",
            ),
        );
    }
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let network =
        match crate::agents::NetworkAccess::parse_form(&form.network, &form.network_domains) {
            Ok(network) => network,
            Err(error) => {
                return render_detail_command(
                    graft,
                    PatchStatus::UnprocessableEntity,
                    detail_view(&state, session.0, &record, &record.title, error.message()),
                );
            }
        };
    match state
        .conversations
        .set_network(&record.id, revision, network)
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store conversation network access", error))
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn select_target(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<AccessForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    if state.sessions.busy(&session.0) {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Another command is active in this browser session.",
            ),
        );
    }
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(project) = ProjectId::parse(&form.project).and_then(|id| state.projects.get(&id))
    else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Choose a granted project.",
            ),
        );
    };
    if !project.host_path_is_available() {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "The selected project is unavailable.",
            ),
        );
    }
    match state
        .conversations
        .select_execution_target(&record.id, revision, project.id)
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("select conversation target", error))
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn detach_project(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path((conversation_id, project_id)): Path<(String, String)>,
    Form(form): Form<RevisionForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    if state.sessions.busy(&session.0) {
        return render_detail_command(
            graft,
            PatchStatus::Conflict,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "Another command is active in this browser session.",
            ),
        );
    }
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    let Some(project) = ProjectId::parse(&project_id) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(
                &state,
                session.0,
                &record,
                &record.title,
                "That project reference is invalid.",
            ),
        );
    };
    match state
        .conversations
        .detach_project(&record.id, revision, project)
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("store project context", error))
        }
        Err(ConversationError::Conflict) => {
            let latest = state.conversations.get(&record.id).unwrap_or(record);
            render_detail_command(
                graft,
                PatchStatus::Conflict,
                detail_view(
                    &state,
                    session.0,
                    &latest,
                    &latest.title,
                    ConversationError::Conflict.message(),
                ),
            )
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

async fn rename_conversation(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<RenameForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &form.title, REVISION_MESSAGE),
        );
    };
    match state
        .conversations
        .rename(&record.id, revision, form.title.clone())
    {
        Ok(updated) => render_detail_command(
            graft,
            PatchStatus::Ok,
            detail_view(&state, session.0, &updated, &updated.title, ""),
        ),
        Err(
            error @ (ConversationError::Random
            | ConversationError::Persist
            | ConversationError::Corrupt),
        ) => Err(AppError::new("store conversation", error)),
        Err(ConversationError::Missing) => Ok(responses::command_navigation("/conversations")),
        Err(ConversationError::Conflict) => {
            let latest = state.conversations.get(&record.id).unwrap_or(record);
            render_detail_command(
                graft,
                PatchStatus::Conflict,
                detail_view(
                    &state,
                    session.0,
                    &latest,
                    &latest.title,
                    ConversationError::Conflict.message(),
                ),
            )
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &form.title, error.message()),
        ),
    }
}

async fn delete_conversation(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<RevisionForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_detail_command(
            graft,
            PatchStatus::UnprocessableEntity,
            detail_view(&state, session.0, &record, &record.title, REVISION_MESSAGE),
        );
    };
    if state.sessions.conversation_reserved(record.id) {
        return render_detail_document_error(
            &state,
            session.0,
            graft,
            &record,
            DocumentError::Active,
        );
    }
    match state.conversations.delete(&record.id, revision) {
        Ok(()) | Err(ConversationError::Missing) => {
            state
                .documents
                .disassociate_conversation(record.id)
                .map_err(|error| AppError::new("remove conversation plan associations", error))?;
            Ok(responses::command_navigation("/conversations"))
        }
        Err(
            error @ (ConversationError::Random
            | ConversationError::Persist
            | ConversationError::Corrupt),
        ) => Err(AppError::new("store conversation", error)),
        Err(ConversationError::Conflict) => {
            let latest = state.conversations.get(&record.id).unwrap_or(record);
            render_detail_command(
                graft,
                PatchStatus::Conflict,
                detail_view(
                    &state,
                    session.0,
                    &latest,
                    &latest.title,
                    ConversationError::Conflict.message(),
                ),
            )
        }
        Err(error) => render_detail_command(
            graft,
            status_for(error),
            detail_view(&state, session.0, &record, &record.title, error.message()),
        ),
    }
}

fn render_detail_document_error(
    state: &AppState,
    session: crate::sessions::SessionId,
    graft: PatchGraft,
    record: &ConversationRecord,
    error: DocumentError,
) -> AppResult<Response> {
    render_detail_command(
        graft,
        document_status(error),
        detail_view(state, session, record, &record.title, error.message()),
    )
}

fn render_plan_page(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    document: &PlanDocument,
    revision: u32,
    error: &'static str,
) -> AppResult<Response> {
    let content = state
        .documents
        .content(document, revision)
        .map_err(|error| AppError::new("read plan document", error))?;
    render_plan_page_with_content(state, graft, status, document, revision, content, error)
}

fn render_plan_page_with_content(
    state: &AppState,
    graft: impl Into<GraftRequest>,
    status: PatchStatus,
    document: &PlanDocument,
    revision: u32,
    content: String,
    error: &'static str,
) -> AppResult<Response> {
    let mut view = PlanDocumentPage::from_document(document, revision, content, error)
        .with_context(state, document);
    view.conversation_revision = document
        .associated_conversation
        .and_then(|id| state.conversations.get(&id))
        .map_or(0, |record| record.revision);
    match graft.into() {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(&view.document_title, state, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            &view.document_title,
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::PatchSet::new()
            .title(&view.document_title)
            .with_children("plan-detail", &view.contents())?
            .respond(status)?),
    }
}

fn document_status(error: DocumentError) -> PatchStatus {
    match error {
        DocumentError::Conflict | DocumentError::Missing | DocumentError::Active => {
            PatchStatus::Conflict
        }
        _ => PatchStatus::UnprocessableEntity,
    }
}

fn plan_secret(state: &AppState, fields: &[&str]) -> Option<String> {
    state
        .preferences
        .desk_providers(&state.vault)
        .into_iter()
        .find_map(|provider| {
            let connection = state.vault.connection_for(&ModelSelection {
                provider: provider.kind,
                model: provider.model,
                thinking: provider.thinking,
            })?;
            let secret = connection.api_key.expose();
            (!secret.is_empty() && fields.iter().any(|field| field.contains(secret)))
                .then(|| secret.to_owned())
        })
}

fn safe_filename(title: &str) -> String {
    let filename: String = title
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect();
    if filename.is_empty() {
        "plan".to_owned()
    } else {
        filename
    }
}

fn effective_model(
    state: &AppState,
    record: &ConversationRecord,
) -> Option<ConversationModelConfiguration> {
    record.model.clone().or_else(|| {
        state
            .preferences
            .desk_providers(&state.vault)
            .into_iter()
            .find(|provider| provider.selected)
            .and_then(|connection| {
                default_environment(state).ok().map(|environment| {
                    ConversationModelConfiguration::direct(
                        ModelSelection {
                            provider: connection.kind,
                            thinking: state.models_dev.effective_effort(
                                connection.kind,
                                &connection.model,
                                connection.thinking.as_ref(),
                            ),
                            model: connection.model,
                        },
                        environment,
                    )
                })
            })
    })
}

pub(super) fn default_environment(state: &AppState) -> Result<EnvironmentId, &'static str> {
    workflows::alpine_git_id(&state.environments).map_err(|error| error.message())
}

pub(super) fn selected_environment(
    state: &AppState,
    raw: &str,
) -> Result<EnvironmentId, &'static str> {
    let environment = if raw.trim().is_empty() {
        default_environment(state).map_err(|_| "Choose an available environment.")?
    } else {
        EnvironmentId::parse(raw.trim()).ok_or("Choose an available environment.")?
    };
    state
        .environments
        .get(&environment)
        .map(|_| environment)
        .ok_or("Choose an available environment.")
}

pub(super) fn has_pending_review(state: &AppState, conversation: ConversationId) -> bool {
    state
        .workflow_runs
        .for_conversation(&conversation)
        .into_iter()
        .any(|run| {
            run.gates
                .iter()
                .any(|gate| gate.state == crate::workflows::gates::HumanGateState::AwaitingDecision)
        })
}

fn remember_selection(state: &AppState, selection: ModelSelection) -> Result<(), &'static str> {
    state
        .preferences
        .select_settings(selection.provider, selection.model, selection.thinking)
        .map_err(|error| {
            crate::error::trace_operation_failure("store model settings", &error);
            "Power Plant cannot store the model preference."
        })
}

fn valid_selection(state: &AppState, selection: &ModelSelection) -> Result<(), &'static str> {
    if !state.vault.contains(selection.provider) {
        return Err("Choose a stored provider.");
    }
    if state
        .models_dev
        .model(selection.provider, &selection.model)
        .is_none()
    {
        return Err("Choose an available model.");
    }
    match selection.thinking.as_ref() {
        Some(effort)
            if !state
                .models_dev
                .supports(selection.provider, &selection.model, effort) =>
        {
            Err("Choose an available thinking effort.")
        }
        None if !state
            .models_dev
            .efforts(selection.provider, &selection.model)
            .is_empty() =>
        {
            Err("Choose an available thinking effort.")
        }
        _ => Ok(()),
    }
}

fn detail_view(
    state: &AppState,
    session: crate::sessions::SessionId,
    record: &ConversationRecord,
    title: &str,
    error: &'static str,
) -> ConversationDetailView {
    let snapshot = record
        .active_job
        .and_then(|job_id| state.sessions.conversation_job(record.id, job_id))
        .map(|job| job.snapshot());
    let error = if error.is_empty()
        && snapshot
            .as_ref()
            .is_some_and(|job| job.status == crate::sessions::JobStatus::Failed)
    {
        "This operation requires recovery. The conversation remains reserved until a restart reconciles the local records."
    } else {
        error
    };
    let pending_gate = state
        .workflow_runs
        .active_runs()
        .into_iter()
        .find(|run| run.conversation_id == Some(record.id))
        .and_then(|run| {
            let destination = crate::slices::human_gates::application_destination(state, &run);
            page::pending_code_gate(&run, &state.workflow_artefacts, destination)
        });
    let (source_review, linked_reviews, source_candidate_review, linked_candidate_reviews) =
        conversation_links(state, record);
    let latest_loop = state
        .task_loops
        .for_conversation(&record.id)
        .into_iter()
        .next();
    let latest_run = state
        .workflow_runs
        .for_conversation(&record.id)
        .into_iter()
        .next();
    let workflow_progress = match (latest_loop, latest_run) {
        (Some(parent), run)
            if run
                .as_ref()
                .is_none_or(|run| parent.created_at_ms >= run.created_at_ms) =>
        {
            let child = parent
                .current_child()
                .and_then(|child| state.workflow_runs.get(&child));
            Some(page::loop_progress_with_child(
                &parent,
                child.as_ref().is_some_and(|run| {
                    run.gates.iter().any(|gate| {
                        gate.state == crate::workflows::gates::HumanGateState::AwaitingDecision
                    })
                }),
                child.as_ref(),
            ))
        }
        (_, Some(run)) => Some(page::workflow_progress(&run)),
        _ => None,
    };
    ConversationDetailView::from_record_with_gate(
        record,
        ModelSources {
            vault: &state.vault,
            preferences: &state.preferences,
            models: &state.models_dev,
            environments: &state.environments,
            environment_snapshots: &state.environment_snapshots,
            projects: &state.projects.list(),
            documents: &state.documents.list_for_conversation(record.id),
            presets: &state.presets.list(),
        },
        &state.agents.list(),
        snapshot.as_ref(),
        state.sessions.busy(&session) || record.active_job.is_some(),
        title,
        error,
        pending_gate,
        source_review,
        linked_reviews,
        source_candidate_review,
        linked_candidate_reviews,
    )
    .with_access_status(state, session, record)
    .with_pending_host_command(
        record
            .active_job
            .and_then(|job_id| state.host_approvals.pending_for(record.id, job_id)),
        record
            .model
            .as_ref()
            .map(|model| {
                crate::slices::execution_settings::page::host_approval_label(
                    model.settings.host_approval,
                )
            })
            .unwrap_or("Ask each time"),
    )
    .with_workflow_progress(workflow_progress)
    .with_plan_actions(state, record)
}

fn conversation_links(
    state: &AppState,
    record: &ConversationRecord,
) -> (
    Option<ConversationLinkView>,
    Vec<ConversationLinkView>,
    Option<CandidateReviewLinkView>,
    Vec<CandidateReviewLinkView>,
) {
    let link_view = |link: &PlanReviewLink| {
        let title = state.conversations.get(&link.conversation_id).map_or_else(
            || "Conversation unavailable".to_owned(),
            |conversation| conversation.title,
        );
        let plan_title = state
            .documents
            .get(&link.plan.document_id)
            .map_or_else(|| "Plan unavailable".to_owned(), |document| document.title);
        ConversationLinkView {
            title,
            plan_title,
            href: format!("/conversations/{}", link.conversation_id.as_hex()),
            plan_href: format!(
                "/plans/{}?revision={}",
                link.plan.document_id.as_hex(),
                link.plan.revision
            ),
            plan_revision: link.plan.revision,
            content_hash: link.plan.content_hash.as_str(),
        }
    };
    let candidate_link_view = |link: &CandidateReviewLink| CandidateReviewLinkView {
        title: link
            .conversation_id
            .and_then(|id| state.conversations.get(&id))
            .map_or_else(String::new, |conversation| conversation.title),
        href: link
            .conversation_id
            .map_or_else(String::new, |id| format!("/conversations/{}", id.as_hex())),
        run_href: format!("/runs/{}", link.run_id.as_hex()),
        candidate_hash: link.candidate.artefact_hash.as_str(),
        diff_base_hash: link.diff_base.artefact_hash.as_str(),
    };
    let source_review = record.source_review.as_ref().map(link_view);
    let linked_reviews = record.plan_reviews.iter().map(link_view).collect();
    let source_candidate_review = record
        .source_candidate_review
        .as_ref()
        .map(candidate_link_view);
    let linked_candidate_reviews = record
        .candidate_reviews
        .iter()
        .map(candidate_link_view)
        .collect();
    (
        source_review,
        linked_reviews,
        source_candidate_review,
        linked_candidate_reviews,
    )
}

fn default_review_brief() -> &'static str {
    "Review this plan for correctness, missing work, risks and unclear requirements. Return findings and concrete recommendations. Do not change project files."
}

fn review_title(document_title: &str) -> String {
    let mut title = "Review: ".to_owned();
    let remaining = crate::conversations::MAXIMUM_TITLE_BYTES - title.len();
    let end = document_title.floor_char_boundary(remaining.min(document_title.len()));
    title.push_str(&document_title[..end]);
    title
}

fn review_model(
    state: &AppState,
    form: &PlanReviewForm,
) -> Result<ConversationModelConfiguration, &'static str> {
    let selection = if form.preset.trim().is_empty() {
        submitted_selection(state, form)?
    } else {
        let preset = AgentId::parse(form.preset.trim())
            .and_then(|id| state.agents.get(&id))
            .ok_or("Choose an available reviewer preset.")?;
        match preset.selection.clone() {
            Some(selection) => selection,
            None => submitted_selection(state, form)?,
        }
    };
    valid_selection(state, &selection)?;
    let environment =
        default_environment(state).map_err(|_| "The starter environment is unavailable.")?;
    if form.preset.trim().is_empty() {
        Ok(ConversationModelConfiguration::direct(
            selection,
            environment,
        ))
    } else {
        let preset = AgentId::parse(form.preset.trim())
            .and_then(|id| state.agents.get(&id))
            .ok_or("Choose an available reviewer preset.")?;
        Ok(ConversationModelConfiguration::from_agent_snapshot(
            &preset,
            selection,
            environment,
        ))
    }
}

fn submitted_selection(
    state: &AppState,
    form: &PlanReviewForm,
) -> Result<ModelSelection, &'static str> {
    let provider = ProviderKind::parse(form.provider.trim()).ok_or("Choose a stored provider.")?;
    let thinking = if form.thinking.trim().is_empty() {
        None
    } else {
        Some(
            ThinkingEffort::new(form.thinking.clone())
                .ok_or("Choose an available thinking effort.")?,
        )
    };
    let selection = ModelSelection::new(provider, form.model.clone(), thinking)
        .ok_or("Enter a valid model name.")?;
    valid_selection(state, &selection)?;
    Ok(selection)
}

fn review_projects(
    state: &AppState,
    source: &ConversationRecord,
    selected: &[String],
) -> Result<Vec<(ProjectId, u32)>, &'static str> {
    let mut projects = Vec::with_capacity(selected.len());
    for raw in selected {
        let project_id = ProjectId::parse(raw.trim()).ok_or("Choose a valid read-only project.")?;
        if projects.iter().any(|(id, _)| *id == project_id) {
            return Err("Choose each read-only project once.");
        }
        let grant = source
            .grants
            .iter()
            .find(|grant| grant.project_id == project_id)
            .ok_or("Only explicitly granted projects can enter this read-only review.")?;
        let project = state
            .projects
            .get(&project_id)
            .ok_or("The selected read-only project is unavailable.")?;
        if project.revision != grant.project_revision || !project.host_path_is_available() {
            return Err("The selected read-only project changed. Reload this review.");
        }
        projects.push((project_id, grant.project_revision));
    }
    Ok(projects)
}

fn review_view_model(
    state: &AppState,
    source: &ConversationRecord,
    form: Option<&PlanReviewForm>,
) -> (Vec<ProviderOption>, Vec<PresetOption>, String) {
    let selection = form
        .and_then(|form| submitted_selection(state, form).ok())
        .or_else(|| effective_model(state, source).map(|model| model.settings.model));
    let providers = state
        .preferences
        .desk_providers(&state.vault)
        .into_iter()
        .map(|provider| ProviderOption {
            value: provider.kind.as_str(),
            label: provider.kind.label(),
            model: selection
                .as_ref()
                .filter(|selection| selection.provider == provider.kind)
                .map_or(provider.model, |selection| selection.model.clone()),
            thinking: selection
                .as_ref()
                .filter(|selection| selection.provider == provider.kind)
                .and_then(|selection| selection.thinking.as_ref())
                .map(|value| value.as_str().to_owned())
                .unwrap_or_default(),
            selected: selection
                .as_ref()
                .is_some_and(|selection| selection.provider == provider.kind),
        })
        .collect();
    let selected_preset = form.map(|form| form.preset.trim()).unwrap_or_default();
    let presets = state
        .agents
        .list()
        .into_iter()
        .map(|agent| PresetOption {
            id: agent.id.as_hex(),
            name: agent.name.clone(),
            description: agent.selection.as_ref().map_or_else(
                || "Keep the selected direct model".to_owned(),
                |selection| format!("{} · {}", selection.provider.label(), selection.model),
            ),
            selected: agent.id.as_hex() == selected_preset,
        })
        .collect();
    let summary = if let Some(form) = form
        && !form.preset.trim().is_empty()
    {
        state
            .agents
            .list()
            .into_iter()
            .find(|agent| agent.id.as_hex() == form.preset.trim())
            .map_or_else(
                || "Reviewer preset is unavailable".to_owned(),
                |agent| format!("Preset: {}", agent.name),
            )
    } else {
        selection.map_or_else(
            || "Choose a stored provider and model".to_owned(),
            |selection| {
                let effort = selection
                    .thinking
                    .as_ref()
                    .map(|effort| format!(" · Thinking: {}", effort.label()))
                    .unwrap_or_default();
                format!(
                    "Direct model: {} · {}{}",
                    selection.provider.label(),
                    selection.model,
                    effort
                )
            },
        )
    };
    (providers, presets, summary)
}

#[allow(clippy::too_many_arguments)]
fn render_plan_review(
    state: &AppState,
    graft: impl Into<GraftRequest>,
    status: PatchStatus,
    source: &ConversationRecord,
    document: &PlanDocument,
    revision: u32,
    brief: &str,
    error: &'static str,
    form: Option<&PlanReviewForm>,
    _selected_projects: &[(ProjectId, u32)],
) -> AppResult<Response> {
    let content = state
        .documents
        .content(document, revision)
        .map_err(|error| AppError::new("read selected plan", error))?;
    let (providers, presets, reviewer_summary) = review_view_model(state, source, form);
    let selected_projects: Vec<_> = form
        .map(|form| form.read_only_project.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let read_only_projects = source
        .grants
        .iter()
        .filter_map(|grant| {
            let project = state.projects.get(&grant.project_id)?;
            Some(ReviewProjectOption {
                id: grant.project_id.as_hex(),
                name: project.name.clone(),
                access: if project.revision == grant.project_revision
                    && project.host_path_is_available()
                {
                    "Read-only authority · List, Read and Run"
                } else {
                    "Read-only authority needs a fresh project record"
                },
                selected: selected_projects
                    .iter()
                    .any(|selected| *selected == grant.project_id.as_hex()),
            })
        })
        .collect();
    let view = PlanReviewView {
        document_title: format!("Review {} | Power Plant", document.title),
        source_title: source.title.clone(),
        source_id: source.id.as_hex(),
        source_revision: source.revision.to_string(),
        document_id: document.id.as_hex(),
        document_revision: revision,
        content_hash: document
            .revision(revision)
            .expect("selected plan revision")
            .content_hash
            .as_str(),
        content_html: page::reply_html(&content),
        brief: if brief.is_empty() {
            default_review_brief().to_owned()
        } else {
            brief.to_owned()
        },
        reviewer_summary,
        providers,
        presets,
        read_only_projects,
        error,
    };
    match graft.into() {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(&view.document_title, state, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            &view.document_title,
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::PatchSet::new()
            .title(&view.document_title)
            .with_children("plan-review-detail", &view.contents())?
            .respond(status)?),
    }
}

fn plan_origin_matches(
    document: &PlanDocument,
    revision: u32,
    conversation: ConversationId,
) -> bool {
    let Some(revision) = document.revision(revision) else {
        return false;
    };
    match &revision.source {
        PlanSource::Action {
            conversation_id, ..
        }
        | PlanSource::ConversationMessage {
            conversation_id, ..
        }
        | PlanSource::SubmittedText {
            conversation_id, ..
        }
        | PlanSource::DirectoryFile {
            conversation_id, ..
        } => *conversation_id == conversation,
        PlanSource::Correction { previous } => {
            plan_origin_matches(document, previous.revision, conversation)
        }
    }
}

fn load_conversation(state: &AppState, raw: &str) -> Option<ConversationRecord> {
    ConversationId::parse(raw).and_then(|id| state.conversations.get(&id))
}
fn parse_revision(raw: &str) -> Option<u32> {
    raw.parse().ok().filter(|revision| *revision > 0)
}
fn conversation_path(record: &ConversationRecord) -> String {
    format!("/conversations/{}", record.id.as_hex())
}
fn status_for(error: ConversationError) -> PatchStatus {
    match error {
        ConversationError::Conflict | ConversationError::Missing | ConversationError::Active => {
            PatchStatus::Conflict
        }
        _ => PatchStatus::UnprocessableEntity,
    }
}
fn render_catalogue(
    state: &AppState,
    graft: GraftRequest,
    filter: &str,
    query: &str,
    error: &'static str,
) -> AppResult<Response> {
    render_page(
        state,
        graft,
        if error.is_empty() {
            PatchStatus::Ok
        } else {
            PatchStatus::UnprocessableEntity
        },
        page::CATALOGUE_TITLE,
        &CatalogueView::from_records(&state.conversations.list(), filter, query, error),
    )
}

fn creation_error(state: &AppState, error: &'static str) -> AppResult<Response> {
    let view = CatalogueView::from_records(&state.conversations.list(), "", "", error);
    let mut patches = hypergraft::PatchSet::new().title(page::CATALOGUE_TITLE);
    patches.children("chat-main", &view)?;
    patches.replace_location("/conversations")?;
    Ok(patches.respond(PatchStatus::UnprocessableEntity)?)
}
fn render_detail(
    state: &AppState,
    _session: crate::sessions::SessionId,
    graft: GraftRequest,
    status: PatchStatus,
    view: ConversationDetailView,
) -> AppResult<Response> {
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(&view.document_title, state, &view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(
            &view.document_title,
            "chat-main",
            &view,
        )?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "conversation-detail",
            &view.contents(),
        )?),
    }
}
fn render_detail_command(
    _graft: PatchGraft,
    status: PatchStatus,
    view: ConversationDetailView,
) -> AppResult<Response> {
    Ok(hypergraft::PatchSet::new()
        .title(&view.document_title)
        .with_children("conversation-detail", &view.contents())?
        .respond(status)?)
}
fn render_page<T: askama::Template>(
    state: &AppState,
    graft: GraftRequest,
    status: PatchStatus,
    title: &str,
    view: &T,
) -> AppResult<Response> {
    match graft {
        GraftRequest::Document => {
            let mut response = responses::chat_page_response(title, state, view)?;
            responses::apply_patch_status(&mut response, status);
            Ok(response)
        }
        GraftRequest::Navigation => Ok(hypergraft::outcome::page_patch(title, "chat-main", view)?),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "chat-main",
            view,
        )?),
    }
}
