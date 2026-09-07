use axum::http::StatusCode;
use tower::ServiceExt;

use crate::providers::ProviderKind;

use super::super::tests::{app, command, connected, test_state, text};

#[tokio::test]
async fn settings_update_validates_the_complete_form_and_revision() {
    let state = test_state();
    let token = connected(&state);
    super::super::tests::ready_starter_environment(&state).await;
    let record = state.conversations.create("Saved".to_owned()).unwrap();
    let effort = state
        .models_dev
        .effective_effort(ProviderKind::Xai, "grok-4.6", None)
        .unwrap();
    let (environment, _) = state
        .environments
        .create(crate::environments::EnvironmentDraft {
            name: "Rust tools".to_owned(),
            oci_image: "docker.io/library/alpine:3.20".to_owned(),
            setup_script: String::new(),
        })
        .unwrap();
    let path = format!("/conversations/{}/settings", record.id);
    let valid = format!(
        "revision={}&provider=xai&model=grok-4.6&thinking={}&instructions={}&tool_read=read&tool_list=list&environment={}&network=restricted&network_domains=example.com",
        record.revision,
        effort.as_str(),
        "Answer%20with%20concise%20evidence.",
        environment.id
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
    assert_eq!(settings.environment, environment.id);
    assert_eq!(
        settings.tools,
        vec![crate::agents::ToolId::List, crate::agents::ToolId::Read]
    );
    assert_eq!(
        settings.network,
        crate::agents::NetworkAccess::Restricted(vec!["example.com".to_owned()])
    );
    state
        .environments
        .delete(&environment.id, environment.revision)
        .unwrap();

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
        format!(
            "revision={}&provider=xai&model=grok-4.6&thinking={}&environment={}",
            updated.revision,
            effort.as_str(),
            environment.id
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

    let owner = super::super::tests::session_id(&token);
    let job = state
        .sessions
        .begin_conversation_job(&owner, updated.id, 1)
        .unwrap();
    state
        .conversations
        .begin_message(
            &updated.id,
            updated.revision,
            settings.model.clone(),
            job.id(),
            "Question".to_owned(),
        )
        .unwrap();
    let active = state.conversations.get(&updated.id).unwrap();
    let response = app(&state)
        .oneshot(command(
            &path,
            &token,
            &format!(
                "revision={}&provider=xai&model=grok-4.6&thinking={}&environment={}",
                active.revision,
                effort.as_str(),
                super::super::default_environment(&state).unwrap()
            ),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("Stop task and switch"));
    assert_eq!(state.conversations.get(&active.id).unwrap(), active);

    let preview_path = format!("/conversations/{}/settings/environment", active.id);
    let response = app(&state)
        .oneshot(command(
            &preview_path,
            &token,
            &format!(
                "revision={}&environment={}",
                record.revision,
                super::super::default_environment(&state).unwrap()
            ),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(text(response).await.contains("Reload the conversation"));

    let (unready, _) = state
        .environments
        .create(crate::environments::EnvironmentDraft {
            name: "Not prepared".to_owned(),
            oci_image: "docker.io/library/alpine:3.20".to_owned(),
            setup_script: String::new(),
        })
        .unwrap();
    let response = app(&state)
        .oneshot(command(
            &preview_path,
            &token,
            &format!("revision={}&environment={}", active.revision, unready.id),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(text(response).await.contains("not ready"));

    let stop_path = format!(
        "/conversations/{}/settings/environment/stop-and-switch",
        active.id
    );
    let other_job = crate::sessions::JobId::generate().unwrap();
    let response = app(&state)
        .oneshot(command(
            &stop_path,
            &token,
            &format!(
                "revision={}&environment={}&job={}",
                active.revision,
                super::super::default_environment(&state).unwrap(),
                other_job
            ),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(!job.cancel_requested());

    let response = app(&state)
        .oneshot(command(
            &stop_path,
            &token,
            &format!(
                "revision={}&environment={}&job={}",
                active.revision,
                super::super::default_environment(&state).unwrap(),
                job.id()
            ),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(text(response).await.contains("still stopping the task"));
    assert_eq!(state.conversations.get(&active.id).unwrap(), active);

    job.finish(crate::sessions::JobStatus::Failed, Some("Cleanup failed"));
    let response = app(&state)
        .oneshot(command(
            &stop_path,
            &token,
            &format!(
                "revision={}&environment={}&job={}",
                active.revision,
                super::super::default_environment(&state).unwrap(),
                job.id()
            ),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(text(response).await.contains("could not clean up the task"));
    assert_eq!(state.conversations.get(&active.id).unwrap(), active);
    assert!(state.sessions.busy(&owner));
}
