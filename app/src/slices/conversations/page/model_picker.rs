use super::{ModelsDevCatalogue, ProviderOption, ProviderVault};

#[derive(Clone, serde::Serialize)]
pub(super) struct EffortOption {
    pub(super) value: String,
    pub(super) label: String,
}

#[derive(Clone, serde::Serialize)]
pub(super) struct ModelOption {
    pub(super) id: String,
    efforts: Vec<EffortOption>,
    default_effort: String,
}

pub(super) struct ModelPicker {
    pub(super) providers: Vec<ProviderOption>,
    pub(super) catalogue: String,
    pub(super) models: Vec<ModelOption>,
    pub(super) efforts: Vec<EffortOption>,
    pub(super) model: String,
    pub(super) thinking: String,
}

impl ModelPicker {
    pub(super) fn new(
        vault: &ProviderVault,
        preferences: &crate::preferences::Preferences,
        catalogue: &ModelsDevCatalogue,
        provider: &str,
        model: &str,
        thinking: &str,
    ) -> Self {
        let connections = preferences.desk_providers(vault);
        let catalogue_models: std::collections::BTreeMap<_, Vec<_>> = connections
            .iter()
            .map(|connection| {
                let models = catalogue
                    .models(connection.kind)
                    .into_iter()
                    .map(|model| ModelOption {
                        efforts: catalogue
                            .efforts(connection.kind, &model.id)
                            .into_iter()
                            .map(|effort| EffortOption {
                                value: effort.as_str().to_owned(),
                                label: effort.label().to_owned(),
                            })
                            .collect(),
                        default_effort: catalogue
                            .effective_effort(
                                connection.kind,
                                &model.id,
                                connection.thinking.as_ref(),
                            )
                            .map(|effort| effort.as_str().to_owned())
                            .unwrap_or_default(),
                        id: model.id,
                    })
                    .collect();
                (connection.kind.as_str(), models)
            })
            .collect();
        let models = catalogue_models.get(provider).cloned().unwrap_or_default();
        let efforts = models
            .iter()
            .find(|option| option.id == model)
            .map(|option| option.efforts.clone())
            .unwrap_or_default();
        Self {
            catalogue: serde_json::to_string(&catalogue_models)
                .expect("catalogue options contain only strings"),
            providers: connections
                .into_iter()
                .map(|connection| ProviderOption {
                    value: connection.kind.as_str(),
                    label: connection.kind.label(),
                    selected: connection.kind.as_str() == provider,
                    model: connection.model,
                    thinking: String::new(),
                })
                .collect(),
            models,
            efforts,
            model: model.to_owned(),
            thinking: thinking.to_owned(),
        }
    }
}
