mod page;

use crate::{
    agents::{NetworkAccess, ToolId},
    environments::EnvironmentId,
    error::AppResult,
    execution::{DirectoryAccess, DirectoryGrant, ExecutionSettings},
    presets::{PresetId, PresetProvenance, PresetRecord},
    providers::{ModelSelection, ProviderKind, ThinkingEffort},
    responses,
    sessions::OptionalSession,
    state::AppState,
};
use axum::{
    Form, Router,
    extract::{Query, State, rejection::FormRejection},
    response::Response,
    routing::{get, post},
};
use hypergraft::{PageGraft, PatchGraft, PatchStatus};
use serde::Deserialize;

// Origin-protected preset commands authorise no execution and need no connected provider.
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/presets", get(show))
        .route("/presets/create", post(create))
        .route("/presets/edit", post(edit))
        .route("/presets/delete", post(delete))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PresetForm {
    preset_id: String,
    revision: String,
    name: String,
    provider: String,
    model: String,
    thinking: String,
    instructions: String,
    tool_list: String,
    tool_read: String,
    tool_write: String,
    tool_run: String,
    environment: String,
    network: String,
    network_domains: String,
    read_only: String,
    reviewed: String,
    direct_write: String,
}

impl PresetForm {
    fn byte_len(&self) -> usize {
        [
            &self.preset_id,
            &self.revision,
            &self.name,
            &self.provider,
            &self.model,
            &self.thinking,
            &self.instructions,
            &self.tool_list,
            &self.tool_read,
            &self.tool_write,
            &self.tool_run,
            &self.environment,
            &self.network,
            &self.network_domains,
            &self.read_only,
            &self.reviewed,
            &self.direct_write,
        ]
        .into_iter()
        .map(String::len)
        .sum()
    }

    fn tools(&self) -> Vec<String> {
        [
            &self.tool_list,
            &self.tool_read,
            &self.tool_write,
            &self.tool_run,
        ]
        .into_iter()
        .filter(|v| !v.is_empty())
        .cloned()
        .collect()
    }

    fn settings(&self, original: Option<&PresetRecord>) -> Result<ExecutionSettings, &'static str> {
        let provider = ProviderKind::parse(&self.provider).ok_or("Choose a listed provider.")?;
        let thinking = if self.thinking.is_empty() {
            None
        } else {
            Some(
                ThinkingEffort::new(self.thinking.clone())
                    .ok_or("Enter a valid reasoning effort.")?,
            )
        };
        let model = ModelSelection::new(provider, self.model.clone(), thinking)
            .ok_or("Enter a valid model name.")?;
        let environment =
            EnvironmentId::parse(&self.environment).ok_or("Choose an environment.")?;
        let tools = self
            .tools()
            .iter()
            .map(|v| ToolId::parse(v).ok_or("Choose only available tools."))
            .collect::<Result<Vec<_>, _>>()?;
        let network = NetworkAccess::parse_form(&self.network, &self.network_domains)
            .map_err(|_| "Enter valid network access and domains.")?;
        if self.read_only.len() + self.reviewed.len() + self.direct_write.len() > 32 * 1024 {
            return Err("Directory paths exceed 32 KiB.");
        }
        let retained = original
            .map(|record| {
                record
                    .settings
                    .directories
                    .iter()
                    .filter(|grant| {
                        [&self.read_only, &self.reviewed, &self.direct_write]
                            .into_iter()
                            .flat_map(|text| text.lines())
                            .any(|path| std::path::Path::new(path) == grant.host_path)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut reserved = retained.clone();
        let mut directories = Vec::new();
        for (text, access) in [
            (&self.read_only, DirectoryAccess::ReadOnly),
            (&self.reviewed, DirectoryAccess::ReviewBeforeApply),
            (&self.direct_write, DirectoryAccess::DirectWrite),
        ] {
            for path in text.lines().filter(|v| !v.trim().is_empty()) {
                let existing = original.and_then(|record| {
                    record
                        .settings
                        .directories
                        .iter()
                        .find(|grant| grant.host_path == std::path::Path::new(path))
                });
                let mut grant = match existing {
                    Some(grant) => grant.clone(),
                    None => {
                        let grant = DirectoryGrant::from_selected(
                            std::path::Path::new(path),
                            &reserved,
                        )
                        .map_err(
                            |_| "Choose existing, distinct directories without overlapping roots.",
                        )?;
                        reserved.push(grant.clone());
                        grant
                    }
                };
                grant.access = access;
                directories.push(grant);
            }
        }
        directories.sort_by_key(|grant| {
            retained
                .iter()
                .position(|old| old.id == grant.id)
                .unwrap_or(usize::MAX)
        });
        ExecutionSettings::new(model, self.instructions.clone(), tools, environment)
            .and_then(|s| s.with_network(network))
            .and_then(|s| s.with_directories(directories))
            .ok_or("Use bounded instructions, unique tools and at most eight distinct directory roots.")
    }
}

#[derive(Default, Deserialize)]
struct Selection {
    #[serde(default)]
    edit: String,
}

async fn show(
    State(state): State<AppState>,
    _session: OptionalSession,
    graft: PageGraft,
    Query(query): Query<Selection>,
) -> AppResult<Response> {
    let record = PresetId::parse(&query.edit).and_then(|id| state.presets.get(&id));
    let error = if !query.edit.is_empty() && record.is_none() {
        "That preset is no longer available."
    } else {
        ""
    };
    let page = page::PresetsPage::new(
        &state,
        record.as_ref().map(PresetForm::from).unwrap_or_default(),
        error,
    );
    match graft {
        PageGraft::Document => responses::chat_page_response("Presets", &state, &page),
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            "Presets",
            "chat-main",
            &page,
        )?),
    }
}

fn patch(
    state: &AppState,
    form: PresetForm,
    error: &'static str,
    status: PatchStatus,
) -> AppResult<Response> {
    Ok(hypergraft::outcome::children_patch(
        status,
        "chat-main",
        &page::PresetsPage::new(state, form, error),
    )?)
}

async fn create(
    State(state): State<AppState>,
    _session: OptionalSession,
    _graft: PatchGraft,
    form: Result<Form<PresetForm>, FormRejection>,
) -> AppResult<Response> {
    save(&state, form, false).await
}

async fn edit(
    State(state): State<AppState>,
    _session: OptionalSession,
    _graft: PatchGraft,
    form: Result<Form<PresetForm>, FormRejection>,
) -> AppResult<Response> {
    save(&state, form, true).await
}

async fn save(
    state: &AppState,
    form: Result<Form<PresetForm>, FormRejection>,
    editing: bool,
) -> AppResult<Response> {
    let Ok(Form(form)) = form else {
        return patch(
            state,
            PresetForm::default(),
            "The preset form is malformed.",
            PatchStatus::UnprocessableEntity,
        );
    };
    let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
        return patch(
            state,
            form,
            crate::local_data::HOST_PATH_RESET_PENDING,
            PatchStatus::Conflict,
        );
    };
    if form.byte_len() > 96 * 1024 {
        return patch(
            state,
            PresetForm::default(),
            "The preset form exceeds 96 KiB.",
            PatchStatus::UnprocessableEntity,
        );
    }
    let original = PresetId::parse(&form.preset_id).and_then(|id| state.presets.get(&id));
    if editing
        && original
            .as_ref()
            .is_none_or(|record| Some(record.revision) != form.revision.parse().ok())
    {
        return patch(
            state,
            form,
            "That preset changed or disappeared. Reload it before you save.",
            PatchStatus::Conflict,
        );
    }
    let settings = match form.settings(original.as_ref()) {
        Ok(settings) => settings,
        Err(error) => return patch(state, form, error, PatchStatus::UnprocessableEntity),
    };
    let unchanged_model = original
        .as_ref()
        .is_some_and(|record| record.settings.model == settings.model);
    if !unchanged_model
        && state
            .models_dev
            .model(settings.model.provider, &settings.model.model)
            .is_some()
    {
        let efforts = state
            .models_dev
            .efforts(settings.model.provider, &settings.model.model);
        if settings
            .model
            .thinking
            .as_ref()
            .map_or(!efforts.is_empty(), |effort| !efforts.contains(effort))
        {
            return patch(
                state,
                form,
                "Choose a reasoning effort that this model supports.",
                PatchStatus::UnprocessableEntity,
            );
        }
    }
    let result = if editing {
        let record = original.expect("revision validation requires a record");
        state
            .presets
            .update(record.id, record.revision, &form.name, settings)
    } else {
        state
            .presets
            .create(&form.name, settings, PresetProvenance::Draft)
    };
    match result {
        Ok(record) => Ok(responses::command_navigation(&format!(
            "/presets?edit={}",
            record.id
        ))),
        Err(error) => patch(
            state,
            form,
            error.message(),
            PatchStatus::UnprocessableEntity,
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteForm {
    preset_id: String,
    revision: u32,
}

async fn delete(
    State(state): State<AppState>,
    _session: OptionalSession,
    _graft: PatchGraft,
    form: Result<Form<DeleteForm>, FormRejection>,
) -> AppResult<Response> {
    let Ok(Form(form)) = form else {
        return patch(
            &state,
            PresetForm::default(),
            "The deletion form is malformed.",
            PatchStatus::UnprocessableEntity,
        );
    };
    let Ok(_permit) = state.local_data.begin_host_path_mutation().await else {
        return patch(
            &state,
            PresetForm::default(),
            crate::local_data::HOST_PATH_RESET_PENDING,
            PatchStatus::Conflict,
        );
    };
    let result = PresetId::parse(&form.preset_id)
        .ok_or(crate::presets::PresetError::Missing)
        .and_then(|id| state.presets.delete(id, form.revision));
    match result {
        Ok(()) => Ok(responses::command_navigation("/presets")),
        Err(error) => patch(
            &state,
            PresetForm::default(),
            error.message(),
            PatchStatus::Conflict,
        ),
    }
}

impl From<&PresetRecord> for PresetForm {
    fn from(record: &PresetRecord) -> Self {
        let s = &record.settings;
        let paths = |access| {
            s.directories
                .iter()
                .filter(|g| g.access == access)
                .map(|g| g.host_path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let tool = |t| {
            if s.tools.contains(&t) {
                t.as_str().to_owned()
            } else {
                String::new()
            }
        };
        Self {
            preset_id: record.id.as_hex(),
            revision: record.revision.to_string(),
            name: record.name.clone(),
            provider: s.model.provider.as_str().to_owned(),
            model: s.model.model.clone(),
            thinking: s
                .model
                .thinking
                .as_ref()
                .map(|e| e.as_str().to_owned())
                .unwrap_or_default(),
            instructions: s.instructions.clone(),
            tool_list: tool(ToolId::List),
            tool_read: tool(ToolId::Read),
            tool_write: tool(ToolId::Write),
            tool_run: tool(ToolId::Run),
            environment: s.environment.as_hex(),
            network: s.network.as_str().to_owned(),
            network_domains: s.network.domains().join("\n"),
            read_only: paths(DirectoryAccess::ReadOnly),
            reviewed: paths(DirectoryAccess::ReviewBeforeApply),
            direct_write: paths(DirectoryAccess::DirectWrite),
        }
    }
}

#[cfg(test)]
mod tests;
