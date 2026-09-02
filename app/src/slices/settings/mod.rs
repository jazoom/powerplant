mod page;

#[cfg(test)]
mod tests;

use axum::{
    Form, Router,
    extract::{State, rejection::FormRejection},
    response::Response,
    routing::{get, post},
};
use hypergraft::{PageGraft, PatchGraft, PatchStatus};
use serde::Deserialize;

use crate::{
    error::AppResult, preferences::Theme, responses, sessions::RequiredSession, state::AppState,
};

use self::page::{SettingsPage, ThemeSetting};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/settings", get(show))
        .route("/settings/theme", post(update_theme))
}

async fn show(
    State(state): State<AppState>,
    _session: RequiredSession,
    graft: PageGraft,
) -> AppResult<Response> {
    let page = SettingsPage::new(state.preferences.theme());
    match graft {
        PageGraft::Document => responses::chat_page_response(page::TITLE, &state, &page),
        PageGraft::Navigation => Ok(hypergraft::outcome::page_patch(
            page::TITLE,
            "chat-main",
            &page,
        )?),
    }
}

#[derive(Deserialize)]
struct ThemeForm {
    theme: String,
}

async fn update_theme(
    State(state): State<AppState>,
    _session: RequiredSession,
    _graft: PatchGraft,
    form: Result<Form<ThemeForm>, FormRejection>,
) -> AppResult<Response> {
    let Ok(Form(form)) = form else {
        return theme_patch(
            PatchStatus::UnprocessableEntity,
            &state,
            Some("Choose a listed theme."),
        );
    };
    let Some(theme) = Theme::parse(&form.theme) else {
        return theme_patch(
            PatchStatus::UnprocessableEntity,
            &state,
            Some("Choose a listed theme."),
        );
    };
    if let Err(error) = state.preferences.set_theme(theme) {
        crate::error::trace_operation_failure("store theme preference", &error);
        return theme_patch(
            PatchStatus::UnprocessableEntity,
            &state,
            Some("Power Plant could not save the theme. Try again."),
        );
    }
    theme_patch(PatchStatus::Ok, &state, None)
}

fn theme_patch(
    status: PatchStatus,
    state: &AppState,
    error: Option<&'static str>,
) -> AppResult<Response> {
    Ok(hypergraft::outcome::children_patch(
        status,
        "theme-setting",
        &ThemeSetting::new(state.preferences.theme(), error),
    )?)
}
