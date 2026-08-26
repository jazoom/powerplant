use axum::Router;

use crate::state::AppState;

mod agents;
mod chat;
mod connect;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .merge(connect::router())
        .merge(agents::router())
        .merge(chat::router())
}
