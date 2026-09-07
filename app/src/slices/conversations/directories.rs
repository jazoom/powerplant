use axum::{
    Form,
    extract::{Path, State},
    response::Response,
};
use hypergraft::{GraftRequest, PatchGraft, PatchStatus};

use crate::{
    conversations::ConversationError,
    error::{AppError, AppResult},
    execution::{DirectoryGrant, DirectoryGrantError, DirectoryGrantId, FolderPick},
    local_data::HOST_PATH_RESET_PENDING,
    responses,
    sessions::RequiredSession,
    state::AppState,
};

use super::{
    REVISION_MESSAGE, detail_view, load_conversation, parse_revision, render_detail,
    render_detail_command, status_for,
};

#[derive(serde::Deserialize)]
pub(super) struct DirectoryForm {
    revision: String,
}

pub(super) async fn pick_new(
    State(state): State<AppState>,
    session: RequiredSession,
    _graft: PatchGraft,
    Form(mut form): Form<super::new::NewForm>,
) -> AppResult<Response> {
    let result = state.folder_picker.pick().await;
    let (status, error) = match result {
        FolderPick::Busy => (
            PatchStatus::Conflict,
            "Wait until the directory chooser closes.",
        ),
        FolderPick::Cancelled => (PatchStatus::Ok, ""),
        FolderPick::Selected(path) => {
            let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
                return render_new(
                    &state,
                    session.0,
                    form,
                    PatchStatus::Conflict,
                    HOST_PATH_RESET_PENDING,
                );
            };
            let existing = match form.directories() {
                Ok(existing) => existing,
                Err(error) => {
                    return render_new(
                        &state,
                        session.0,
                        form,
                        PatchStatus::UnprocessableEntity,
                        error,
                    );
                }
            };
            match DirectoryGrant::from_selected(&path, &existing) {
                Ok(grant) if sensitive(&state, &grant) => (
                    PatchStatus::UnprocessableEntity,
                    DirectoryGrantError::Sensitive.message(),
                ),
                Ok(grant) => {
                    let mut directories = existing;
                    directories.push(grant);
                    form.set_directories(&directories);
                    (PatchStatus::Ok, "")
                }
                Err(error) => (directory_status(error), error.message()),
            }
        }
    };
    render_new(&state, session.0, form, status, error)
}

pub(super) async fn remove_new(
    State(state): State<AppState>,
    session: RequiredSession,
    _graft: PatchGraft,
    Path(grant_id): Path<String>,
    Form(mut form): Form<super::new::NewForm>,
) -> AppResult<Response> {
    let Some(grant_id) = DirectoryGrantId::parse(&grant_id) else {
        return render_new(
            &state,
            session.0,
            form,
            PatchStatus::UnprocessableEntity,
            DirectoryGrantError::Invalid.message(),
        );
    };
    let directories = match form.directories() {
        Ok(directories) => directories,
        Err(error) => {
            return render_new(
                &state,
                session.0,
                form,
                PatchStatus::UnprocessableEntity,
                error,
            );
        }
    };
    if !directories.iter().any(|grant| grant.id == grant_id) {
        return render_new(
            &state,
            session.0,
            form,
            PatchStatus::UnprocessableEntity,
            DirectoryGrantError::Invalid.message(),
        );
    }
    let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
        return render_new(
            &state,
            session.0,
            form,
            PatchStatus::Conflict,
            HOST_PATH_RESET_PENDING,
        );
    };
    let retained = directories
        .into_iter()
        .filter(|grant| grant.id != grant_id)
        .collect::<Vec<_>>();
    form.set_directories(&retained);
    render_new(&state, session.0, form, PatchStatus::Ok, "")
}

pub(super) async fn pick_saved(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<DirectoryForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let Some(revision) = parse_revision(&form.revision) else {
        return render_saved(
            &state,
            session.0,
            graft,
            &record,
            PatchStatus::UnprocessableEntity,
            REVISION_MESSAGE,
        );
    };
    match state.folder_picker.pick().await {
        FolderPick::Busy => render_saved(
            &state,
            session.0,
            graft,
            &record,
            PatchStatus::Conflict,
            "Wait until the directory chooser closes.",
        ),
        FolderPick::Cancelled => {
            render_saved(&state, session.0, graft, &record, PatchStatus::Ok, "")
        }
        FolderPick::Selected(path) => {
            let existing = record
                .model
                .as_ref()
                .map(|model| model.settings.directories.as_slice())
                .unwrap_or_default();
            let grant = match DirectoryGrant::from_selected(&path, existing) {
                Ok(grant) if sensitive(&state, &grant) => {
                    return render_saved(
                        &state,
                        session.0,
                        graft,
                        &record,
                        PatchStatus::UnprocessableEntity,
                        DirectoryGrantError::Sensitive.message(),
                    );
                }
                Ok(grant) => grant,
                Err(error) => {
                    return render_saved(
                        &state,
                        session.0,
                        graft,
                        &record,
                        directory_status(error),
                        error.message(),
                    );
                }
            };
            let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
                return render_saved(
                    &state,
                    session.0,
                    graft,
                    &record,
                    PatchStatus::Conflict,
                    HOST_PATH_RESET_PENDING,
                );
            };
            match state
                .conversations
                .add_directory(&record.id, revision, grant)
            {
                Ok(updated) => {
                    render_saved(&state, session.0, graft, &updated, PatchStatus::Ok, "")
                }
                Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
                    Err(AppError::new("store conversation directory", error))
                }
                Err(error) => render_saved(
                    &state,
                    session.0,
                    graft,
                    &record,
                    status_for(error),
                    error.message(),
                ),
            }
        }
    }
}

pub(super) async fn remove_saved(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path((conversation_id, grant_id)): Path<(String, String)>,
    Form(form): Form<DirectoryForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    let (Some(revision), Some(grant_id)) = (
        parse_revision(&form.revision),
        DirectoryGrantId::parse(&grant_id),
    ) else {
        return render_saved(
            &state,
            session.0,
            graft,
            &record,
            PatchStatus::UnprocessableEntity,
            REVISION_MESSAGE,
        );
    };
    let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
        return render_saved(
            &state,
            session.0,
            graft,
            &record,
            PatchStatus::Conflict,
            HOST_PATH_RESET_PENDING,
        );
    };
    match state
        .conversations
        .remove_directory(&record.id, revision, grant_id)
    {
        Ok(updated) => render_saved(&state, session.0, graft, &updated, PatchStatus::Ok, ""),
        Err(error @ (ConversationError::Persist | ConversationError::Corrupt)) => {
            Err(AppError::new("remove conversation directory", error))
        }
        Err(error) => render_saved(
            &state,
            session.0,
            graft,
            &record,
            status_for(error),
            error.message(),
        ),
    }
}

fn sensitive(state: &AppState, grant: &DirectoryGrant) -> bool {
    crate::execution::authority::sensitive_directory(&grant.host_path, state.local_data.root())
}

fn directory_status(error: DirectoryGrantError) -> PatchStatus {
    match error {
        DirectoryGrantError::Full => PatchStatus::Conflict,
        _ => PatchStatus::UnprocessableEntity,
    }
}

fn render_new(
    state: &AppState,
    session: crate::sessions::SessionId,
    form: super::new::NewForm,
    status: PatchStatus,
    error: &'static str,
) -> AppResult<Response> {
    render_detail(
        state,
        session,
        GraftRequest::Patch,
        status,
        super::page::ConversationDetailView::from_new(state, form, error).open_directories(),
    )
}

fn render_saved(
    state: &AppState,
    session: crate::sessions::SessionId,
    graft: PatchGraft,
    record: &crate::conversations::ConversationRecord,
    status: PatchStatus,
    error: &'static str,
) -> AppResult<Response> {
    render_detail_command(
        graft,
        status,
        detail_view(state, session, record, &record.title, error).open_directories(),
    )
}

#[cfg(test)]
mod tests;
