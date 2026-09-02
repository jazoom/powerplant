use askama::Template;

use crate::preferences::Theme;

pub(super) const TITLE: &str = "Settings | Power Plant";

#[derive(Template)]
#[template(path = "settings/templates/index.html")]
pub(super) struct SettingsPage {
    theme: &'static str,
    themes: &'static [Theme],
    error: Option<&'static str>,
}

impl SettingsPage {
    pub(super) fn new(theme: Theme) -> Self {
        Self {
            theme: theme.as_str(),
            themes: Theme::ALL,
            error: None,
        }
    }
}

#[derive(Template)]
#[template(path = "settings/templates/theme.html")]
pub(super) struct ThemeSetting {
    theme: &'static str,
    themes: &'static [Theme],
    error: Option<&'static str>,
}

impl ThemeSetting {
    pub(super) fn new(theme: Theme, error: Option<&'static str>) -> Self {
        Self {
            theme: theme.as_str(),
            themes: Theme::ALL,
            error,
        }
    }
}
