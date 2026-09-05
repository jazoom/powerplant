use std::{fs, io, path::PathBuf, sync::Mutex};

use serde::{Deserialize, Serialize};

const FILE_VERSION: u32 = 1;
const MAXIMUM_FILE_BYTES: usize = 1024;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Theme {
    #[default]
    Springfield,
    EvergreenTerrace,
    Leftorium,
    Stonecutters,
    Sector7G,
}

impl Theme {
    pub(crate) const ALL: &[Self] = &[
        Self::Springfield,
        Self::EvergreenTerrace,
        Self::Leftorium,
        Self::Stonecutters,
        Self::Sector7G,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Springfield => "springfield",
            Self::EvergreenTerrace => "evergreen-terrace",
            Self::Leftorium => "leftorium",
            Self::Stonecutters => "stonecutters",
            Self::Sector7G => "sector-7-g",
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Springfield => "Springfield",
            Self::EvergreenTerrace => "Evergreen Terrace",
            Self::Leftorium => "Leftorium",
            Self::Stonecutters => "Stonecutters",
            Self::Sector7G => "Sector 7-G",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|theme| theme.as_str() == value)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PreferencesFile {
    version: u32,
    theme: String,
    show_thinking: bool,
}

#[derive(Clone, Copy, Default)]
struct PreferenceValues {
    theme: Theme,
    show_thinking: bool,
}

#[derive(Debug)]
pub(crate) struct PreferenceError;

impl std::fmt::Display for PreferenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("preference persist failed")
    }
}

impl std::error::Error for PreferenceError {}

pub(crate) struct Preferences {
    path: Option<PathBuf>,
    values: Mutex<PreferenceValues>,
}

impl Preferences {
    pub(crate) fn open(path: PathBuf) -> Self {
        Self {
            values: Mutex::new(load(&path)),
            path: Some(path),
        }
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            values: Mutex::new(PreferenceValues::default()),
        }
    }

    pub(crate) fn theme(&self) -> Theme {
        self.values().theme
    }

    pub(crate) fn set_theme(&self, theme: Theme) -> Result<(), PreferenceError> {
        self.update(|values| values.theme = theme)
    }

    pub(crate) fn show_thinking(&self) -> bool {
        self.values().show_thinking
    }

    pub(crate) fn set_show_thinking(&self, show: bool) -> Result<(), PreferenceError> {
        self.update(|values| values.show_thinking = show)
    }

    fn values(&self) -> PreferenceValues {
        *self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, change: impl FnOnce(&mut PreferenceValues)) -> Result<(), PreferenceError> {
        let mut current = self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut next = *current;
        change(&mut next);
        if let Some(path) = self.path.as_deref() {
            persist(path, next)?;
        }
        *current = next;
        Ok(())
    }
}

fn load(path: &std::path::Path) -> PreferenceValues {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return PreferenceValues::default();
        }
        Err(_) => return PreferenceValues::default(),
        Ok(_) => {}
    }
    let Ok(bytes) = crate::storage::read_private_bounded(path, MAXIMUM_FILE_BYTES) else {
        return PreferenceValues::default();
    };
    let Ok(file) = serde_json::from_slice::<PreferencesFile>(&bytes) else {
        return PreferenceValues::default();
    };
    if file.version != FILE_VERSION {
        return PreferenceValues::default();
    }
    let Some(theme) = Theme::parse(&file.theme) else {
        return PreferenceValues::default();
    };
    PreferenceValues {
        theme,
        show_thinking: file.show_thinking,
    }
}

fn persist(path: &std::path::Path, values: PreferenceValues) -> Result<(), PreferenceError> {
    let file = PreferencesFile {
        version: FILE_VERSION,
        theme: values.theme.as_str().to_owned(),
        show_thinking: values.show_thinking,
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|_| PreferenceError)?;
    let dir = path.parent().ok_or(PreferenceError)?;
    crate::storage::ensure_private_dir(dir).map_err(|_| PreferenceError)?;
    crate::storage::write_private(path, &bytes).map_err(|_| PreferenceError)
}
