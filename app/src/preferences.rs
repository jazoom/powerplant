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
    theme: Mutex<Theme>,
}

impl Preferences {
    pub(crate) fn open(path: PathBuf) -> Self {
        Self {
            theme: Mutex::new(load(&path)),
            path: Some(path),
        }
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            theme: Mutex::new(Theme::default()),
        }
    }

    pub(crate) fn theme(&self) -> Theme {
        *self
            .theme
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn set_theme(&self, theme: Theme) -> Result<(), PreferenceError> {
        let mut current = self
            .theme
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(path) = self.path.as_deref() {
            persist(path, theme)?;
        }
        *current = theme;
        Ok(())
    }
}

fn load(path: &std::path::Path) -> Theme {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Theme::default(),
        Err(_) => return Theme::default(),
        Ok(_) => {}
    }
    let Ok(bytes) = crate::storage::read_private_bounded(path, MAXIMUM_FILE_BYTES) else {
        return Theme::default();
    };
    let Ok(file) = serde_json::from_slice::<PreferencesFile>(&bytes) else {
        return Theme::default();
    };
    if file.version != FILE_VERSION {
        return Theme::default();
    }
    Theme::parse(&file.theme).unwrap_or_default()
}

fn persist(path: &std::path::Path, theme: Theme) -> Result<(), PreferenceError> {
    let file = PreferencesFile {
        version: FILE_VERSION,
        theme: theme.as_str().to_owned(),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|_| PreferenceError)?;
    let dir = path.parent().ok_or(PreferenceError)?;
    crate::storage::ensure_private_dir(dir).map_err(|_| PreferenceError)?;
    crate::storage::write_private(path, &bytes).map_err(|_| PreferenceError)
}
