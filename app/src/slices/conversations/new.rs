use super::*;

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct NewForm {
    pub(super) project: String,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) thinking: String,
    pub(super) preset: String,
    pub(super) preset_preview: String,
    pub(super) preset_name: String,
    pub(super) instructions: String,
    pub(super) tool_list: String,
    pub(super) tool_read: String,
    pub(super) tool_write: String,
    pub(super) tool_run: String,
    pub(super) environment: String,
    pub(super) network: String,
    pub(super) network_domains: String,
    pub(super) directory_0: String,
    pub(super) directory_1: String,
    pub(super) directory_2: String,
    pub(super) directory_3: String,
    pub(super) directory_4: String,
    pub(super) directory_5: String,
    pub(super) directory_6: String,
    pub(super) directory_7: String,
    pub(super) draft_nonce: String,
    pub(super) consent_reference: String,
    pub(super) pending_directory: String,
    pub(super) consent_request: String,
    pub(super) consent_existing: String,
    pub(super) title: String,
    pub(super) message: String,
    pub(super) action: String,
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
        environment: super::default_environment(&state)
            .map(|id| id.as_hex())
            .unwrap_or_default(),
        draft_nonce: crate::execution::draft_nonce().map_err(|_| {
            AppError::new(
                "create draft consent nonce",
                std::io::Error::other("system random source unavailable"),
            )
        })?,
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
        ConversationDetailView::from_new(&state, session.0, form, error),
    )
}

impl NewForm {
    pub(super) fn consent_nonce(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        // Bind incomplete draft fields without a provider prerequisite for directory selection.
        for value in [
            &self.project,
            &self.provider,
            &self.model,
            &self.thinking,
            &self.preset,
            &self.instructions,
            &self.tool_list,
            &self.tool_read,
            &self.tool_write,
            &self.tool_run,
            &self.environment,
            &self.network,
            &self.network_domains,
        ] {
            digest.update(value.len().to_le_bytes());
            digest.update(value);
        }
        format!(
            "{}:{}",
            self.draft_nonce,
            crate::hex::encode(&digest.finalize())
        )
    }

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

    pub(super) fn directories(
        &self,
    ) -> Result<Vec<crate::execution::DirectoryGrant>, &'static str> {
        let directories = self
            .directory_values()
            .into_iter()
            .map(|value| {
                crate::execution::DirectoryGrant::parse_form(value)
                    .ok_or("A directory grant is not valid.")
            })
            .collect::<Result<Vec<_>, _>>()?;
        crate::execution::validate_directories(&directories)
            .map_err(|_| "Choose valid non-overlapping directories.")?;
        Ok(directories)
    }

    fn directory_values(&self) -> Vec<&str> {
        [
            &self.directory_0,
            &self.directory_1,
            &self.directory_2,
            &self.directory_3,
            &self.directory_4,
            &self.directory_5,
            &self.directory_6,
            &self.directory_7,
        ]
        .into_iter()
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .collect()
    }

    pub(super) fn pending_directory(&self) -> Option<crate::execution::DirectoryGrant> {
        (!self.pending_directory.is_empty())
            .then(|| crate::execution::DirectoryGrant::parse_form(&self.pending_directory))
            .flatten()
    }

    pub(super) fn set_directories(&mut self, directories: &[crate::execution::DirectoryGrant]) {
        let mut values = directories.iter().map(|grant| grant.form_value());
        for field in [
            &mut self.directory_0,
            &mut self.directory_1,
            &mut self.directory_2,
            &mut self.directory_3,
            &mut self.directory_4,
            &mut self.directory_5,
            &mut self.directory_6,
            &mut self.directory_7,
        ] {
            *field = values.next().unwrap_or_default();
        }
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

pub(super) fn model(
    state: &AppState,
    session: crate::sessions::SessionId,
    form: &NewForm,
) -> Result<Option<ConversationModelConfiguration>, &'static str> {
    let configuration = settings_snapshot(state, session, form)?;
    if let Some(configuration) = &configuration {
        valid_selection(state, &configuration.settings.model)?;
    }
    Ok(configuration)
}

pub(super) fn settings_snapshot(
    state: &AppState,
    session: crate::sessions::SessionId,
    form: &NewForm,
) -> Result<Option<ConversationModelConfiguration>, &'static str> {
    if form.provider.is_empty() && form.model.is_empty() {
        return Ok(None);
    }
    let provider = ProviderKind::parse(&form.provider).ok_or("Choose a stored provider.")?;
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
    let environment = if form.environment.trim().is_empty() {
        super::default_environment(state).map_err(|_| "Choose an environment.")?
    } else {
        EnvironmentId::parse(form.environment.trim()).ok_or("Choose a valid environment.")?
    };
    let network_mode = if form.network.trim().is_empty() {
        "none"
    } else {
        form.network.as_str()
    };
    let network = crate::agents::NetworkAccess::parse_form(network_mode, &form.network_domains)
        .map_err(|_| "Choose valid network access. Restricted access needs 1 to 32 domains.")?;
    let settings = crate::execution::ExecutionSettings::new(
        selection,
        form.instructions.clone(),
        super::settings::parse_tools(&form.tool_values())?,
        environment,
    )
    .and_then(|settings| settings.with_network(network))
    .and_then(|settings| settings.with_directories(form.directories().ok()?))
    .ok_or("Enter valid conversation settings.")?;
    let preset = state
        .presets
        .applied_draft(session, &form.preset_preview)
        .filter(|preset| preset.id.as_hex() == form.preset && preset.settings == settings);
    Ok(Some(match preset {
        Some(preset) => ConversationModelConfiguration::from_preset(&preset),
        None => ConversationModelConfiguration {
            settings,
            preset: None,
        },
    }))
}

pub(super) async fn remember_model(
    State(state): State<AppState>,
    session: RequiredSession,
    _graft: PatchGraft,
    Form(mut form): Form<NewForm>,
) -> AppResult<Response> {
    form.preset.clear();
    form.preset_preview.clear();
    let (status, message) = match model(&state, session.0, &form) {
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
            ConversationDetailView::from_new(&state, session.0, form, error),
        )
    };
    let reject_settings = |status, error, form| {
        render_detail(
            &state,
            session.0,
            GraftRequest::Patch,
            status,
            ConversationDetailView::from_new(&state, session.0, form, error).open_settings(),
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
    let model = match model(&state, session.0, &form) {
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
    if form.pending_directory().is_some() {
        return reject(
            PatchStatus::UnprocessableEntity,
            "Approve or remove the pending sensitive directory.",
            form,
        );
    }
    let consent_grants = model
        .settings
        .directories
        .iter()
        .filter(|grant| {
            grant.access == crate::execution::DirectoryAccess::ReviewBeforeApply
                || crate::execution::authority::sensitive_directory(
                    &grant.host_path,
                    state.local_data.root(),
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    if consent_grants.iter().any(|grant| {
        !state.sessions.contains_live(&session.0)
            || !state.access_consent.authorised_draft(
                &form.consent_reference,
                session.0,
                &form.consent_nonce(),
                &model.settings.directories,
                grant,
            )
    }) {
        return reject(
            PatchStatus::UnprocessableEntity,
            "Directory review or sensitive access needs explicit approval.",
            form,
        );
    }
    if let Err(error) = super::preflight_execution(&state, session.0, None, &model).await {
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
    let Ok(permit) = state.local_data.begin_host_path_mutation().await else {
        return reject(
            PatchStatus::Conflict,
            crate::local_data::HOST_PATH_RESET_PENDING,
            form,
        );
    };
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
    if !consent_grants.is_empty()
        && state
            .access_consent
            .consume_draft(
                &form.consent_reference,
                session.0,
                &form.consent_nonce(),
                &model.settings,
                record.id,
                &consent_grants,
            )
            .is_err()
    {
        let _ = state.conversations.delete(&id, 1);
        return reject(
            PatchStatus::UnprocessableEntity,
            "Directory review or sensitive access needs explicit approval.",
            form,
        );
    }
    drop(permit);
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
