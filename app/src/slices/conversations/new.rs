use super::*;

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct NewForm {
    pub(super) project: String,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) thinking: String,
    pub(super) preset: String,
    pub(super) instructions: String,
    pub(super) tool_list: String,
    pub(super) tool_read: String,
    pub(super) tool_write: String,
    pub(super) tool_run: String,
    pub(super) network: String,
    pub(super) network_domains: String,
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
        network: "none".to_owned(),
        ..NewForm::default()
    };
    if let Some(provider) = state
        .preferences
        .desk_providers(&state.vault)
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

impl NewForm {
    pub(super) fn tool_values(&self) -> Vec<String> {
        [
            &self.tool_list,
            &self.tool_read,
            &self.tool_write,
            &self.tool_run,
        ]
        .into_iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect()
    }
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
    let mut configuration = match preset {
        Some(preset) => ConversationModelConfiguration::from_preset(&preset, selection),
        None => ConversationModelConfiguration::direct(selection),
    };
    if configuration.preset.is_none() {
        let tools = super::settings::parse_tools(&form.tool_values())?;
        let network_mode = if form.network.trim().is_empty() {
            "none"
        } else {
            form.network.as_str()
        };
        let network = crate::agents::NetworkAccess::parse_form(network_mode, &form.network_domains)
            .map_err(|_| "Choose valid network access. Restricted access needs 1 to 32 domains.")?;
        configuration.settings = crate::execution::ExecutionSettings::new(
            configuration.settings.model,
            form.instructions.clone(),
            tools,
        )
        .and_then(|settings| settings.with_network(network))
        .ok_or("Enter instructions within 32 KiB without unsupported control characters.")?;
    }
    Ok(Some(configuration))
}

pub(super) async fn remember_model(
    State(state): State<AppState>,
    _session: RequiredSession,
    _graft: PatchGraft,
    Form(mut form): Form<NewForm>,
) -> AppResult<Response> {
    form.preset.clear();
    let (status, message) = match model(&state, &form) {
        Ok(Some(model)) => match remember_selection(&state, model.settings.model) {
            Ok(()) => (PatchStatus::Ok, ""),
            Err(error) => (PatchStatus::UnprocessableEntity, error),
        },
        Ok(None) => (
            PatchStatus::UnprocessableEntity,
            "Choose a stored provider.",
        ),
        Err(error) => (PatchStatus::UnprocessableEntity, error),
    };
    Ok(hypergraft::outcome::children_patch(
        status,
        "conversation-model-status",
        &page::ModelSelectionStatus { message },
    )?)
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
    let reject_settings = |status, error, form| {
        render_detail(
            &state,
            session.0,
            GraftRequest::Patch,
            status,
            ConversationDetailView::from_new(&state, form, error).open_settings(),
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
        Err(error) => {
            return reject_settings(PatchStatus::UnprocessableEntity, error, form);
        }
    };
    if model
        .as_ref()
        .and_then(|model| state.vault.connection_for(&model.settings.model))
        .is_none()
    {
        return reject_settings(
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
    let Some(model) = model else {
        return reject_settings(
            PatchStatus::UnprocessableEntity,
            "Choose a stored provider.",
            form,
        );
    };
    if let Err(error) = super::preflight_execution(&state, &model).await {
        let super::StartMessageError::User(status, message) = error else {
            return Err(AppError::new(
                "preflight first conversation message",
                std::io::Error::other("unexpected internal preflight error"),
            ));
        };
        return reject_settings(status, message, form);
    }
    let id = ConversationId::generate()
        .map_err(|error| AppError::new("create conversation identifier", error))?;
    let record = match state.conversations.create_saved(
        id,
        project,
        (!form.title.is_empty()).then(|| form.title.clone()),
        Some(model.clone()),
        None,
    ) {
        Ok(record) => record,
        Err(error) => return reject(status_for(error), error.message(), form),
    };
    let record =
        match super::start_message(&state, session.0, record, 1, model, form.message.clone()).await
        {
            Ok(record) => record,
            Err(super::StartMessageError::Internal(error)) => return Err(error),
            Err(super::StartMessageError::User(status, error)) => {
                let _ = state.conversations.delete(&id, 1);
                return reject(status, error, form);
            }
        };
    let warning = record
        .model
        .as_ref()
        .and_then(|model| remember_selection(&state, model.settings.model.clone()).err())
        .unwrap_or("");
    let view = detail_view(&state, session.0, &record, &record.title, warning);
    let mut patches = hypergraft::PatchSet::new();
    patches
        .children("conversation-detail", &view.contents())
        .map_err(|error| AppError::new("render first conversation save", error))?;
    patches
        .replace_location(conversation_path(&record))
        .map_err(|error| AppError::new("locate first conversation save", error))?;
    patches = patches.title(&view.document_title);
    patches
        .respond(PatchStatus::Ok)
        .map_err(|error| AppError::new("respond to first conversation save", error))
}

#[cfg(test)]
mod tests;
