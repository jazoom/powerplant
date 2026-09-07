use super::*;

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct NewForm {
    pub(super) project: String,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) thinking: String,
    pub(super) preset: String,
    pub(super) title: String,
    pub(super) message: String,
    action: String,
}

pub(super) async fn show(
    State(state): State<AppState>,
    session: RequiredSession,
    graft: GraftRequest,
    Query(query): Query<CatalogueQuery>,
) -> AppResult<Response> {
    let mut form = NewForm {
        project: query.project,
        ..NewForm::default()
    };
    if let Some(provider) = state
        .vault
        .desk_providers()
        .into_iter()
        .find(|provider| provider.selected)
    {
        form.provider = provider.kind.as_str().to_owned();
        form.thinking = state
            .models_dev
            .effective_effort(provider.kind, &provider.model, provider.thinking.as_ref())
            .map(|effort| effort.as_str().to_owned())
            .unwrap_or_default();
        form.model = provider.model;
    }
    let error = project(&state, &form.project).err().unwrap_or("");
    render_detail(
        &state,
        session.0,
        graft,
        PatchStatus::Ok,
        ConversationDetailView::from_new(&state, form, error),
    )
}

fn project(state: &AppState, raw: &str) -> Result<Option<ProjectId>, &'static str> {
    if raw.is_empty() {
        return Ok(None);
    }
    ProjectId::parse(raw)
        .filter(|id| state.projects.get(id).is_some())
        .map(Some)
        .ok_or("Choose an available project.")
}

fn model(
    state: &AppState,
    form: &NewForm,
) -> Result<Option<ConversationModelConfiguration>, &'static str> {
    if form.provider.is_empty() && form.model.is_empty() && form.preset.is_empty() {
        return Ok(None);
    }
    let preset = if form.preset.is_empty() {
        None
    } else {
        Some(
            AgentId::parse(&form.preset)
                .and_then(|id| state.agents.get(&id))
                .ok_or("Choose an available preset.")?,
        )
    };
    let selection = if let Some(selection) =
        preset.as_ref().and_then(|preset| preset.selection.clone())
    {
        selection
    } else {
        let provider = ProviderKind::parse(&form.provider).ok_or("Choose a stored provider.")?;
        let thinking = if form.thinking.trim().is_empty() {
            None
        } else {
            Some(
                ThinkingEffort::new(form.thinking.clone())
                    .ok_or("Choose an available thinking effort.")?,
            )
        };
        ModelSelection::new(provider, form.model.clone(), thinking)
            .ok_or("Enter a valid model name.")?
    };
    valid_selection(state, &selection)?;
    Ok(Some(match preset {
        Some(preset) => ConversationModelConfiguration::from_preset(&preset, selection),
        None => ConversationModelConfiguration::direct(selection),
    }))
}

pub(super) async fn save(
    State(state): State<AppState>,
    session: RequiredSession,
    _graft: PatchGraft,
    Form(form): Form<NewForm>,
) -> AppResult<Response> {
    let reject = |status, error, form| {
        render_detail(
            &state,
            session.0,
            GraftRequest::Patch,
            status,
            ConversationDetailView::from_new(&state, form, error),
        )
    };
    if form.action != "send" {
        return reject(
            PatchStatus::UnprocessableEntity,
            "Send a message to start the conversation.",
            form,
        );
    }
    let project = match project(&state, &form.project) {
        Ok(project) => project,
        Err(error) => return reject(PatchStatus::UnprocessableEntity, error, form),
    };
    let model = match model(&state, &form) {
        Ok(model) => model,
        Err(error) => return reject(PatchStatus::UnprocessableEntity, error, form),
    };
    let Some(connection) = model
        .as_ref()
        .and_then(|model| state.vault.connection_for(&model.selection))
    else {
        return reject(
            PatchStatus::UnprocessableEntity,
            "Choose a stored provider.",
            form,
        );
    };
    // No identity, reservation or store mutation precedes input validation.
    if form.message.len() > crate::conversations::MAXIMUM_MESSAGE_BYTES
        || crate::conversations::normalise_message(&form.message).is_err()
    {
        return reject(
            PatchStatus::UnprocessableEntity,
            ConversationError::Message.message(),
            form,
        );
    }
    if !form.title.is_empty() && crate::conversations::normalise_title(&form.title).is_err() {
        return reject(
            PatchStatus::UnprocessableEntity,
            ConversationError::Title.message(),
            form,
        );
    }
    let id = ConversationId::generate()
        .map_err(|error| AppError::new("create conversation identifier", error))?;
    let job = match state.sessions.begin_conversation_job(&session.0, id, 1) {
        Ok(job) => job,
        Err(_) => {
            return reject(
                PatchStatus::Conflict,
                "Another command is active in this browser session.",
                form,
            );
        }
    };
    let record = match state.conversations.create_saved(
        id,
        project,
        (!form.title.is_empty()).then(|| form.title.clone()),
        model,
        Some((job.id(), form.message.clone())),
    ) {
        Ok(record) => record,
        Err(error) => {
            state
                .sessions
                .finish_conversation_job(&session.0, id, job.id());
            return reject(status_for(error), error.message(), form);
        }
    };
    let view = detail_view(&state, session.0, &record, &record.title, "");
    let mut patches = hypergraft::PatchSet::new();
    patches
        .children("conversation-detail", &view.contents())
        .map_err(|error| AppError::new("render first conversation save", error))?;
    patches
        .replace_location(conversation_path(&record))
        .map_err(|error| AppError::new("locate first conversation save", error))?;
    patches = patches.title(&view.document_title);
    // New project context grants no authority. The first reply uses no guest tools.
    tokio::spawn(job::run(
        state.clone(),
        session.0,
        id,
        record,
        connection,
        job,
    ));
    patches
        .respond(PatchStatus::Ok)
        .map_err(|error| AppError::new("respond to first conversation save", error))
}

#[cfg(test)]
mod tests;
