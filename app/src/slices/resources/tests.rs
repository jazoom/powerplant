use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

#[tokio::test]
async fn resources_support_document_and_navigation_without_a_provider() {
    let state = crate::tests::test_state(crate::config::RuntimeConfig::development());
    let app = crate::slices::router()
        .layer(from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state);
    for (kind, status) in [
        (None, StatusCode::OK),
        (Some("navigation"), StatusCode::OK),
        (Some("patch"), StatusCode::BAD_REQUEST),
    ] {
        let mut request = Request::builder().uri("/resources");
        if let Some(kind) = kind {
            request = request
                .header(hypergraft::GRAFT_REQUEST, kind)
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
        }
        assert_eq!(
            app.clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            status
        );
    }
}
