use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    middleware::from_fn_with_state,
};
use tower::ServiceExt;

#[tokio::test]
async fn routes_support_navigation_and_patch_commands_only() {
    let state = crate::tests::test_state(crate::config::RuntimeConfig::development());
    let token = crate::sessions::generate_session_token().unwrap();
    state.sessions.insert(token.id());
    let app = crate::slices::router()
        .layer(from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    for (kind, status) in [
        (None, StatusCode::OK),
        (Some("navigation"), StatusCode::OK),
        (Some("patch"), StatusCode::BAD_REQUEST),
    ] {
        let mut request = Request::builder().uri("/presets").header(
            header::COOKIE,
            format!("powerplant_session={}", token.raw().as_str()),
        );
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
    for path in ["/presets/create", "/presets/edit", "/presets/delete"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(
                        header::COOKIE,
                        format!("powerplant_session={}", token.raw().as_str()),
                    )
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("name=Forged"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert!(state.presets.list().is_empty());
    let record = state
        .presets
        .create(
            "Unavailable",
            form().settings(None).unwrap(),
            PresetProvenance::Draft,
        )
        .unwrap();
    let changed = state
        .presets
        .update(
            record.id,
            record.revision,
            "Updated",
            record.settings.clone(),
        )
        .unwrap();
    for (revision, expected) in [
        (record.revision, StatusCode::CONFLICT),
        (changed.revision, StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/presets/delete")
                    .header(hypergraft::GRAFT_REQUEST, "patch")
                    .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!(
                        "preset_id={}&revision={revision}",
                        record.id
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    assert!(state.presets.get(&record.id).is_none());
}

#[tokio::test]
async fn invalid_and_stale_edits_leave_the_saved_snapshot_unchanged() {
    let state = crate::tests::test_state(crate::config::RuntimeConfig::development());
    let record = state
        .presets
        .create(
            "Original",
            form().settings(None).unwrap(),
            PresetProvenance::Draft,
        )
        .unwrap();
    let app = crate::slices::router()
        .layer(from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    for (fields, expected) in [
        ("name=Changed&revision=0".to_owned(), StatusCode::CONFLICT),
        (
            format!("name={}&revision=1", "x".repeat(96 * 1024)),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "name=Changed&revision=1&unknown=value".to_owned(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/presets/edit")
                    .header(hypergraft::GRAFT_REQUEST, "patch")
                    .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!("preset_id={}&{fields}", record.id)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(state.presets.get(&record.id), Some(record.clone()));
    }
}

fn form() -> PresetForm {
    PresetForm {
        name: "Review".to_owned(),
        provider: "xai".to_owned(),
        model: "grok-4.6".to_owned(),
        environment: EnvironmentId::generate().unwrap().as_hex(),
        network: "none".to_owned(),
        ..PresetForm::default()
    }
}

#[test]
fn form_rejects_unknown_duplicate_tools_and_unbounded_instructions() {
    let mut f = form();
    f.tool_read = "unknown".to_owned();
    assert!(f.settings(None).is_err());
    f.tool_read = "read".to_owned();
    f.tool_run = "read".to_owned();
    assert!(f.settings(None).is_err());
    f.tool_run.clear();
    f.instructions = "x".repeat(32769);
    assert!(f.settings(None).is_err());
}

#[test]
fn edits_keep_directory_order_and_avoid_alias_collisions() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("one/src");
    let second = temp.path().join("two/src");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let mut f = form();
    f.reviewed = first.display().to_string();
    let store = crate::presets::PresetStore::in_memory();
    let record = store
        .create(
            "Sources",
            f.settings(None).unwrap(),
            PresetProvenance::Draft,
        )
        .unwrap();
    f.read_only = second.display().to_string();
    let edited = f.settings(Some(&record)).unwrap();
    assert_eq!(edited.directories[0], record.settings.directories[0]);
    assert_ne!(edited.directories[0].alias, edited.directories[1].alias);
}

#[test]
fn directory_paths_retain_trailing_spaces_without_retargeting() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source ");
    std::fs::create_dir(&path).unwrap();
    std::fs::create_dir(temp.path().join("source")).unwrap();
    let mut f = form();
    f.read_only = path.display().to_string();
    let store = crate::presets::PresetStore::in_memory();
    let original = store
        .create(&f.name, f.settings(None).unwrap(), PresetProvenance::Draft)
        .unwrap();
    assert_eq!(original.settings.directories[0].host_path, path);
    std::fs::remove_dir(&path).unwrap();
    let edited = PresetForm::from(&original)
        .settings(Some(&original))
        .unwrap();
    assert_eq!(edited.directories, original.settings.directories);
    assert!(!edited.directories[0].is_available());
}

#[test]
fn unavailable_directory_keeps_identity_and_duplicate_roots_fail() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source");
    std::fs::create_dir(&path).unwrap();
    let mut f = form();
    f.read_only = path.display().to_string();
    let settings = f.settings(None).unwrap();
    let store = crate::presets::PresetStore::in_memory();
    let original = store
        .create(&f.name, settings, PresetProvenance::Draft)
        .unwrap();
    std::fs::remove_dir(&path).unwrap();
    assert_eq!(f.settings(Some(&original)).unwrap(), original.settings);
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        f.settings(Some(&original)).unwrap().directories[0].identity,
        original.settings.directories[0].identity
    );
    f.reviewed = f.read_only.clone();
    assert!(f.settings(Some(&original)).is_err());
}
