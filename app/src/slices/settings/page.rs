use askama::Template;

pub(super) const TITLE: &str = "Settings | Power Plant";

#[derive(Template)]
#[template(path = "settings/templates/index.html")]
pub(super) struct SettingsPage;
