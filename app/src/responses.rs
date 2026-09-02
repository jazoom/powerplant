//! Safe local redirects, generic error bodies and document responses.

use askama::Template;
use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};

use crate::{
    assets::AssetPaths,
    error::{AppResult, AppResultExt},
    security::CspNonce,
};

pub(crate) fn apply_patch_status(response: &mut Response, status: hypergraft::PatchStatus) {
    *response.status_mut() = status.status_code();
    if let hypergraft::PatchStatus::TooManyRequests(retry_after) = status {
        retry_after.apply(response.headers_mut());
    }
}

pub(crate) fn page_redirect(graft: hypergraft::PageGraft, destination: &str) -> Response {
    hypergraft::outcome::page_redirect(graft, destination)
        .unwrap_or_else(|_| no_store_status_response(StatusCode::BAD_REQUEST, "Bad request"))
}

pub(crate) fn command_navigation(destination: &str) -> Response {
    hypergraft::outcome::command_navigation(destination)
        .unwrap_or_else(|_| no_store_status_response(StatusCode::BAD_REQUEST, "Bad request"))
}

pub(crate) fn request_navigation(
    graft: impl Into<hypergraft::GraftRequest>,
    destination: &str,
) -> Response {
    match graft.into() {
        hypergraft::GraftRequest::Document => {
            page_redirect(hypergraft::PageGraft::Document, destination)
        }
        hypergraft::GraftRequest::Navigation => {
            page_redirect(hypergraft::PageGraft::Navigation, destination)
        }
        hypergraft::GraftRequest::Patch => command_navigation(destination),
    }
}

enum FullDocument {
    Connect,
    Chat,
}

impl FullDocument {
    fn operation(&self) -> &'static str {
        match self {
            Self::Connect => "render connect page",
            Self::Chat => "render chat page",
        }
    }
}

fn render_page<T>(
    status: StatusCode,
    title: &str,
    assets: &AssetPaths,
    document: FullDocument,
    content: &T,
) -> AppResult<Response>
where
    T: Template,
{
    let operation = document.operation();
    let render = || -> Result<Response, askama::Error> {
        let nonce = CspNonce::generate();
        let content_html = nonce.render(content)?;
        let html = match document {
            FullDocument::Connect => nonce.render(&ConnectPage {
                title,
                css_path: &assets.css_path,
                js_path: &assets.js_path,
                content: &content_html,
            })?,
            FullDocument::Chat => nonce.render(&ChatPage {
                title,
                css_path: &assets.css_path,
                js_path: &assets.js_path,
                content: &content_html,
            })?,
        };
        Ok(varied_no_store_html(status, html, nonce))
    };
    render().with_operation(operation)
}

pub(crate) fn connect_page_response<T>(
    title: &str,
    assets: &AssetPaths,
    content: &T,
) -> AppResult<Response>
where
    T: Template,
{
    render_page(
        StatusCode::OK,
        title,
        assets,
        FullDocument::Connect,
        content,
    )
}

pub(crate) fn chat_page_response<T>(
    title: &str,
    assets: &AssetPaths,
    content: &T,
) -> AppResult<Response>
where
    T: Template,
{
    render_page(StatusCode::OK, title, assets, FullDocument::Chat, content)
}

pub(crate) fn internal_error_response() -> Response {
    no_store_status_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
}

pub(crate) fn no_store_status_response(status: StatusCode, body: &'static str) -> Response {
    let mut response = (status, body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(crate) fn origin_failure_response() -> Response {
    no_store_status_response(StatusCode::FORBIDDEN, "Forbidden.")
}

fn varied_no_store_html(status: StatusCode, html: String, nonce: CspNonce) -> Response {
    let mut response = (status, Html(html)).into_response();
    response.extensions_mut().insert(nonce);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[derive(Template)]
#[template(path = "layout/connect.html")]
struct ConnectPage<'a> {
    title: &'a str,
    css_path: &'a str,
    js_path: &'a str,
    content: &'a str,
}

#[derive(Template)]
#[template(path = "layout/chat.html")]
struct ChatPage<'a> {
    title: &'a str,
    css_path: &'a str,
    js_path: &'a str,
    content: &'a str,
}
