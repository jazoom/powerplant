use axum::http::StatusCode;
use tower::ServiceExt;

use crate::providers::ProviderKind;

use super::super::tests::{app, command, connected, test_state, text};

#[tokio::test]
async fn settings_update_validates_the_complete_form_and_revision() {
    let state = test_state();
    let token = connected(&state);
    let record = state.conversations.create("Saved".to_owned()).unwrap();
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, "grok-4.6", None)
        .unwrap();
    let path = format!("/conversations/{}/settings", record.id);
    let valid = format!(
        "revision={}&provider=xai&model=grok-4.6&thinking={}&instructions={}&tool_read=read&tool_list=list&network=restricted&network_domains=example.com",
        record.revision,
        effort.as_str(),
        "Answer%20with%20concise%20evidence."
    );
    let response = app(&state)
        .oneshot(command(&path, &token, &valid))
        .await
        .unwrap();
    let status = response.status();
    let body = text(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("data-settings-open=\"true\""));
    let updated = state.conversations.get(&record.id).unwrap();
    let settings = &updated.model.as_ref().unwrap().settings;
    assert_eq!(settings.instructions, "Answer with concise evidence.");
    assert_eq!(
        settings.tools,
        vec![crate::agents::ToolId::List, crate::agents::ToolId::Read]
    );
    assert_eq!(
        settings.network,
        crate::agents::NetworkAccess::Restricted(vec!["example.com".to_owned()])
    );

    for fields in [
        format!(
            "revision={}&provider=xai&model=grok-4.6&thinking={}&tool_read=unknown",
            updated.revision,
            effort.as_str()
        ),
        format!(
            "revision={}&provider=xai&model=grok-4.6&thinking=invalid",
            updated.revision
        ),
        format!(
            "revision={}&provider=xai&model=grok-4.6&thinking={}&network=restricted&network_domains=https%3A%2F%2Fexample.com",
            updated.revision,
            effort.as_str()
        ),
    ] {
        let response = app(&state)
            .oneshot(command(&path, &token, &fields))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = text(response).await;
        assert!(body.contains("data-settings-open=\"true\""));
        if fields.contains("thinking=invalid") {
            assert!(body.contains("Unavailable · invalid"));
        }
        assert_eq!(state.conversations.get(&record.id).unwrap(), updated);
    }

    let stale = format!(
        "revision={}&provider=xai&model=grok-4.6&thinking={}&instructions=Retained&tool_read=read",
        record.revision,
        effort.as_str()
    );
    let response = app(&state)
        .oneshot(command(&path, &token, &stale))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = text(response).await;
    assert!(body.contains("Retained"));
    assert!(body.contains("data-settings-open=\"true\""));
}
