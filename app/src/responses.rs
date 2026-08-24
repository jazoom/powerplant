//! Safe local redirects, generic error bodies and document or Hypergraft responses.

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

pub(crate) use hypergraft::{CommandGraft, GraftRequest, PageGraft, PatchSet, PatchStatus};

fn native_patch_status(status: PatchStatus) -> StatusCode {
    match status {
        PatchStatus::Ok => StatusCode::OK,
        PatchStatus::Unauthorized => StatusCode::UNAUTHORIZED,
        PatchStatus::Conflict => StatusCode::CONFLICT,
        PatchStatus::UnprocessableEntity => StatusCode::UNPROCESSABLE_ENTITY,
        PatchStatus::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
    }
}

pub(crate) fn graft_redirect(graft: impl Into<GraftRequest>, destination: &str) -> Response {
    hypergraft::outcome::redirect(graft.into(), destination)
        .unwrap_or_else(|_| no_store_status_response(StatusCode::BAD_REQUEST, "Bad request"))
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

pub(crate) fn connect_graft_page<T, F>(
    graft: GraftRequest,
    status: PatchStatus,
    title: &str,
    assets: &AssetPaths,
    content: &T,
    card_contents: &F,
) -> AppResult<Response>
where
    T: Template,
    F: Template,
{
    match graft {
        GraftRequest::Document => {
            let mut response = connect_page_response(title, assets, content)?;
            *response.status_mut() = native_patch_status(status);
            Ok(response)
        }
        GraftRequest::Navigation if status == PatchStatus::Ok => Ok(
            hypergraft::outcome::page_patch(title, "connect-main", content)?,
        ),
        GraftRequest::Patch => Ok(hypergraft::outcome::children_patch(
            status,
            "connect-card",
            card_contents,
        )?),
        _ => Ok(no_store_status_response(
            StatusCode::BAD_REQUEST,
            "Bad request",
        )),
    }
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

pub(crate) fn chat_graft_page<T>(
    graft: PageGraft,
    title: &str,
    assets: &AssetPaths,
    content: &T,
) -> AppResult<Response>
where
    T: Template,
{
    match graft {
        PageGraft::Document => chat_page_response(title, assets, content),
        PageGraft::Navigation => hypergraft::outcome::page_patch(title, "chat-main", content)
            .with_operation("construct chat page navigation patch"),
    }
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
