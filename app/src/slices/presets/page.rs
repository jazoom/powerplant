use super::PresetForm;
use crate::slices::execution_settings::page::{ToolOption, tool_options};
use crate::{presets::PresetProvenance, providers::ProviderKind, state::AppState};
use askama::Template;

pub(super) struct OptionView {
    value: String,
    label: String,
    selected: bool,
}

pub(super) struct RecordView {
    id: String,
    name: String,
    revision: u32,
    source: String,
}

#[derive(Template)]
#[template(path = "presets/templates/index.html")]
pub(super) struct PresetsPage {
    form: PresetForm,
    records: Vec<RecordView>,
    providers: Vec<OptionView>,
    environments: Vec<OptionView>,
    directories: Vec<String>,
    model_status: &'static str,
    thinking_efforts: String,
    instructions: String,
    tool_options: Vec<ToolOption>,
    job_active: bool,
    error: &'static str,
}

impl PresetsPage {
    pub(super) fn new(state: &AppState, mut form: PresetForm, error: &'static str) -> Self {
        if form.network.is_empty() {
            form.network = "none".to_owned();
        }
        let providers = ProviderKind::ALL
            .into_iter()
            .map(|kind| OptionView {
                value: kind.as_str().to_owned(),
                label: format!(
                    "{}{}",
                    kind.label(),
                    if state.vault.contains(kind) {
                        ""
                    } else {
                        " — Unavailable"
                    }
                ),
                selected: form.provider == kind.as_str(),
            })
            .collect();
        let environments = crate::slices::execution_settings::page::environment_options(
            &state.environments,
            &state.environment_snapshots,
            crate::environments::EnvironmentId::parse(&form.environment),
        )
        .into_iter()
        .map(|option| OptionView {
            value: option.id,
            label: format!(
                "{} — {} · {}",
                option.name, option.readiness, option.availability
            ),
            selected: option.selected,
        })
        .collect();
        let directories = crate::presets::PresetId::parse(&form.preset_id)
            .and_then(|id| state.presets.get(&id))
            .map(|r| {
                r.settings
                    .directories
                    .iter()
                    .map(|g| {
                        format!(
                            "{} · {} · {}",
                            g.host_path.display(),
                            g.guest_path(),
                            if g.is_available() {
                                "Requested access only"
                            } else {
                                "Unavailable"
                            }
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let kind = ProviderKind::parse(&form.provider);
        let model_status = if form.model.is_empty() {
            ""
        } else if kind
            .and_then(|kind| state.models_dev.model(kind, &form.model))
            .is_some()
        {
            "Listed model"
        } else {
            "Model unavailable in the catalogue; requested value retained"
        };
        let efforts = kind
            .map(|kind| state.models_dev.efforts(kind, &form.model))
            .unwrap_or_default();
        let thinking_efforts = efforts
            .iter()
            .map(|effort| effort.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            model_status,
            thinking_efforts,
            instructions: form.instructions.clone(),
            tool_options: tool_options(&form.tools()),
            form,
            records: state
                .presets
                .list()
                .into_iter()
                .map(|r| RecordView {
                    id: r.id.as_hex(),
                    name: r.name,
                    revision: r.revision,
                    source: match r.provenance {
                        PresetProvenance::Conversation { title, .. } => {
                            format!("From conversation: {title}")
                        }
                        PresetProvenance::Draft => "Independent settings".to_owned(),
                    },
                })
                .collect(),
            providers,
            environments,
            directories,
            job_active: false,
            error,
        }
    }

    fn model_form(&self) -> &'static str {
        "preset-editor"
    }
}
