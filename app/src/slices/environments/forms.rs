use crate::environments::{
    EnvironmentDraft, EnvironmentError, EnvironmentRecipeVersion, EnvironmentRecord,
};

pub(super) const MAXIMUM_FORM_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub(super) struct EnvironmentFormState {
    pub(super) name: String,
    pub(super) oci_image: String,
    pub(super) setup_script: String,
    pub(super) revision: Option<u64>,
    pub(super) recipe_version: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FormErrors {
    pub(super) summary: &'static str,
    pub(super) name: &'static str,
    pub(super) image: &'static str,
    pub(super) script: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FormError {
    UnknownField,
    DuplicateField,
    Revision,
    Recipe,
}

impl FormError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::UnknownField => "That form includes an unknown field.",
            Self::DuplicateField => "That form includes a duplicate field.",
            Self::Revision => "Reload the environment and try again.",
            Self::Recipe => "That recipe changed. Reload it.",
        }
    }
}

impl FormErrors {
    pub(super) fn summary(message: &'static str) -> Self {
        Self {
            summary: message,
            ..Self::default()
        }
    }
}

impl From<EnvironmentError> for FormErrors {
    fn from(error: EnvironmentError) -> Self {
        let message = error.message();
        match error {
            EnvironmentError::Name => Self {
                name: message,
                ..Self::default()
            },
            EnvironmentError::Image
            | EnvironmentError::LocalPath
            | EnvironmentError::DiskImage
            | EnvironmentError::Archive => Self {
                image: message,
                ..Self::default()
            },
            EnvironmentError::Script => Self {
                script: message,
                ..Self::default()
            },
            _ => Self::summary(message),
        }
    }
}

impl EnvironmentFormState {
    pub(super) fn blank() -> Self {
        Self {
            name: String::new(),
            oci_image: String::new(),
            setup_script: String::new(),
            revision: None,
            recipe_version: None,
        }
    }

    pub(super) fn parse(pairs: Vec<(String, String)>) -> Result<Self, FormError> {
        let mut state = Self::blank();
        let mut seen = Vec::new();
        for (key, value) in pairs {
            if seen.iter().any(|item| item == &key) {
                return Err(FormError::DuplicateField);
            }
            seen.push(key.clone());
            match key.as_str() {
                "name" => state.name = value,
                "oci_image" => state.oci_image = value,
                "setup_script" => state.setup_script = value,
                "revision" => state.revision = Some(parse_revision(&value)?),
                "recipe_version" => state.recipe_version = Some(value),
                _ => return Err(FormError::UnknownField),
            }
        }
        Ok(state)
    }

    pub(super) fn to_draft(&self) -> EnvironmentDraft {
        EnvironmentDraft {
            name: self.name.clone(),
            oci_image: self.oci_image.clone(),
            setup_script: self.setup_script.clone(),
        }
    }

    pub(super) fn from_record(record: &EnvironmentRecord) -> Self {
        Self {
            name: record.name.clone(),
            oci_image: record.recipe.oci_image.as_str().to_owned(),
            setup_script: record.recipe.setup_script.clone(),
            revision: Some(record.revision),
            recipe_version: Some(record.recipe_version.as_hex()),
        }
    }
}

pub(super) fn parse_delete(pairs: &[(String, String)]) -> Result<(u64, bool), FormError> {
    let mut revision = None;
    let mut confirm = false;
    let mut seen = Vec::new();
    for (key, value) in pairs {
        if seen.contains(&key) {
            return Err(FormError::DuplicateField);
        }
        seen.push(key);
        match key.as_str() {
            "revision" => revision = Some(parse_revision(value)?),
            "confirm" => confirm = is_checked(value),
            _ => return Err(FormError::UnknownField),
        }
    }
    Ok((revision.ok_or(FormError::Revision)?, confirm))
}

pub(super) fn parse_retry(
    pairs: &[(String, String)],
) -> Result<(u64, EnvironmentRecipeVersion), FormError> {
    let mut revision = None;
    let mut recipe = None;
    let mut seen = Vec::new();
    for (key, value) in pairs {
        if seen.contains(&key) {
            return Err(FormError::DuplicateField);
        }
        seen.push(key);
        match key.as_str() {
            "revision" => revision = Some(parse_revision(value)?),
            "recipe_version" => {
                recipe =
                    Some(EnvironmentRecipeVersion::parse(value.trim()).ok_or(FormError::Recipe)?);
            }
            _ => return Err(FormError::UnknownField),
        }
    }
    Ok((
        revision.ok_or(FormError::Revision)?,
        recipe.ok_or(FormError::Recipe)?,
    ))
}

fn parse_revision(value: &str) -> Result<u64, FormError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(FormError::Revision);
    }
    value.parse().map_err(|_| FormError::Revision)
}

fn is_checked(value: &str) -> bool {
    matches!(value, "on" | "true" | "1")
}

#[cfg(test)]
mod tests;
