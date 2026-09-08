use super::*;
use crate::{
    agents::{AccessMode, NetworkAccess, ToolId},
    sandbox::{CommandEvent, GuestExec, MountSpec, SandboxSpec},
    workflows::task_list,
};
use base64::Engine;

#[cfg(test)]
mod tests;

const IMPORT_SCRIPT: &str = "p=.; rest=$1; while :; do part=${rest%%/*}; p=$p/$part; [ ! -L \"$p\" ] || exit 1; case $rest in */*) rest=${rest#*/};; *) break;; esac; done; [ -f \"$p\" ] && [ -r \"$p\" ] || exit 1; exec 3<\"$p\" || exit 1; resolved=$(readlink -f /proc/self/fd/3) || exit 1; case $resolved in \"$2\"/*) ;; *) exit 1;; esac; head -c 65537 <&3 | base64";

#[derive(Deserialize)]
pub(super) struct ImportForm {
    revision: String,
    directory_id: String,
    path: String,
    title: String,
}

pub(super) async fn import(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: PatchGraft,
    Path(conversation_id): Path<String>,
    Form(form): Form<ImportForm>,
) -> AppResult<Response> {
    let Some(record) = load_conversation(&state, &conversation_id) else {
        return Ok(responses::command_navigation("/conversations"));
    };
    if parse_revision(&form.revision) != Some(record.revision) || record.active_job.is_some() {
        return render_detail_document_error(
            &state,
            session.0,
            graft,
            &record,
            DocumentError::Conflict,
        );
    }
    let reserved =
        state
            .sessions
            .begin_conversation_job(&session.0, record.id, record.messages.len());
    let Ok(job) = reserved else {
        return render_detail_document_error(
            &state,
            session.0,
            graft,
            &record,
            DocumentError::Active,
        );
    };
    let worker_state = state.clone();
    let worker_record = record.clone();
    // The detached worker owns cleanup if the browser disconnects during guest access.
    let result = tokio::spawn(async move {
        let result = import_file(&worker_state, session.0, &worker_record, form).await;
        worker_state
            .sessions
            .finish_conversation_job(&session.0, worker_record.id, job.id());
        result
    })
    .await
    .map_err(|error| AppError::new("import task list worker", error))?;
    match result {
        Ok(()) => Ok(responses::command_navigation(&conversation_path(&record))),
        Err(error) => {
            let current = state.conversations.get(&record.id).unwrap_or(record);
            render_detail_command(
                graft,
                PatchStatus::UnprocessableEntity,
                detail_view(&state, session.0, &current, &current.title, error),
            )
        }
    }
}

async fn import_file(
    state: &AppState,
    session: crate::sessions::SessionId,
    record: &ConversationRecord,
    form: ImportForm,
) -> Result<(), &'static str> {
    if !task_list::valid_project_path(&form.path) {
        return Err("Select a directory-relative file path without parent components.");
    }
    let _permit = state
        .local_data
        .begin_host_path_mutation()
        .await
        .map_err(|_| "Local data reset blocks file import.")?;
    let grant = import_grant(state, session, record, &form.directory_id)?;
    let _execution = state
        .workflow_execution
        .acquire()
        .map_err(|_| "Another workflow is active.")?;
    let environment = record
        .model
        .as_ref()
        .ok_or("Select conversation settings first.")?
        .settings
        .environment;
    let pinned = workflows::pin_quick_task_with_context(
        AccessMode::ReadOnly,
        &[ToolId::Read],
        "",
        environment,
        Vec::new(),
    )
    .map_err(|error| error.message())?;
    let environments = workflows::resolve_environments(
        &pinned.definition,
        &state.environments,
        &state.environment_snapshots,
    )
    .await
    .map_err(|error| error.message())?;
    let environment = environments
        .environments
        .first()
        .ok_or("The environment is unavailable.")?;
    let snapshot = state
        .environment_snapshots
        .restore_path(&environment.snapshot.artifact_key)
        .map_err(|_| "The environment snapshot is unavailable.")?;
    let run = workflows::RunId::generate().map_err(|_| "The import identity is unavailable.")?;
    let attempt =
        workflows::AttemptId::generate().map_err(|_| "The import identity is unavailable.")?;
    let sandbox = state.sandboxes.attempt_handle(run, attempt);
    let spec = SandboxSpec {
        mounts: vec![MountSpec {
            guest: grant.guest_path(),
            host: grant.host_path.clone(),
            read_only: true,
        }],
        workdir: grant.guest_path(),
        network: NetworkAccess::None,
    };
    let result = async {
        let current = state
            .conversations
            .get(&record.id)
            .ok_or("The conversation is unavailable.")?;
        if current.revision != record.revision {
            return Err("The conversation changed. Reload before import.");
        }
        import_grant(state, session, &current, &form.directory_id)?;
        sandbox
            .start_from_snapshot(
                &snapshot,
                environment.snapshot.snapshot_digest.as_str(),
                spec,
            )
            .await
            .map_err(|error| error.message())?;
        read_file(&sandbox, &form.path, &grant.guest_path()).await
    }
    .await;
    if sandbox.stop().await.is_ok() && sandbox.remove().await.is_ok() {
        state.sandboxes.drop_attempt(attempt);
    } else {
        state.sandboxes.expose_orphan(sandbox.name().to_owned());
        return Err("The import sandbox cleanup failed. The task list was not saved.");
    }
    let markdown = result?;
    let current = state
        .conversations
        .get(&record.id)
        .ok_or("The conversation is unavailable.")?;
    if current.revision != record.revision {
        return Err("The conversation changed. Reload before import.");
    }
    import_grant(state, session, &current, &form.directory_id)?;
    let secret = plan_secret(state, &[&form.title, &form.path, &markdown]);
    state
        .documents
        .create_task_list(
            record.id,
            form.title,
            &markdown,
            PlanSource::DirectoryFile {
                conversation_id: record.id,
                directory_id: grant.id,
                path: form.path,
                source_hash: workflows::artefacts::ObjectHash::of(markdown.as_bytes()),
            },
            secret.as_deref(),
        )
        .map_err(|error| error.message())?;
    Ok(())
}

fn import_grant<'a>(
    state: &AppState,
    session: crate::sessions::SessionId,
    record: &'a ConversationRecord,
    directory_id: &str,
) -> Result<&'a crate::execution::DirectoryGrant, &'static str> {
    let settings = &record
        .model
        .as_ref()
        .ok_or("Select conversation settings first.")?
        .settings;
    let id = crate::execution::DirectoryGrantId::parse(directory_id)
        .ok_or("Select an authorised directory.")?;
    let grant = settings
        .directories
        .iter()
        .find(|grant| grant.id == id)
        .ok_or("Grant access to the selected directory first.")?;
    grant.revalidate().map_err(|error| error.message())?;
    if !state.sessions.contains_live(&session) {
        return Err("The session expired. Reload before import.");
    }
    if (grant.access != crate::execution::DirectoryAccess::ReadOnly
        || crate::execution::authority::sensitive_directory(
            &grant.host_path,
            state.local_data.root(),
        ))
        && !state
            .access_consent
            .authorised_conversation(session, record.id, settings, grant)
    {
        return Err("Directory access needs approval. Open Directories before import.");
    }
    Ok(grant)
}

async fn read_file(
    sandbox: &crate::sandbox::GuestSandbox,
    path: &str,
    root: &str,
) -> Result<String, &'static str> {
    // Positional arguments keep submitted paths out of shell syntax. Each component rejects symlinks.
    let request = GuestExec::command(
        "sh",
        vec![
            "-c".to_owned(),
            IMPORT_SCRIPT.to_owned(),
            "task-import".to_owned(),
            path.to_owned(),
            root.to_owned(),
        ],
    );
    let mut command = sandbox
        .exec_cmd(request)
        .await
        .map_err(|_| "The directory file is unreadable.")?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut encoded = String::new();
        let mut exit = None;
        while let Some(event) = command.recv().await {
            match event {
                CommandEvent::Output(piece) => {
                    if encoded.len().saturating_add(piece.len()) > 96 * 1024 {
                        return Err("The directory file exceeds the task-list limit.");
                    }
                    encoded.push_str(&piece);
                }
                CommandEvent::Exited(code) => exit = Some(code),
                CommandEvent::Failed => return Err("The directory file is unreadable."),
            }
        }
        if exit != Some(0) {
            return Err("The directory file is unreadable.");
        }
        encoded.retain(|character| !character.is_ascii_whitespace());
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "The directory file is unreadable.")?;
        if bytes.len() > task_list::MAXIMUM_TASK_LIST_BYTES {
            return Err("The directory file exceeds the task-list limit.");
        }
        String::from_utf8(bytes).map_err(|_| "The directory file is not valid UTF-8 text.")
    })
    .await;
    if !matches!(result, Ok(Ok(_))) {
        command.kill().await;
    }
    command.close().await;
    result.map_err(|_| "The directory file read timed out.")?
}
