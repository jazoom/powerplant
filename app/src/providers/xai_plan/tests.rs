use super::PlanFile;

#[test]
fn proxy_headers_identify_a_supported_grok_cli() {
    let headers = super::proxy_headers();
    assert_eq!(
        headers
            .get("x-grok-client-version")
            .and_then(|value| value.to_str().ok()),
        Some("0.2.116")
    );
    assert_eq!(
        headers
            .get("x-grok-client-identifier")
            .and_then(|value| value.to_str().ok()),
        Some("grok-shell")
    );
    assert_eq!(
        headers
            .get(reqwest::header::USER_AGENT)
            .and_then(|value| value.to_str().ok()),
        Some("xai-grok-cli")
    );
}

#[test]
fn plan_file_debug_redacts_tokens() {
    let file = PlanFile {
        access_token: "xai-plan-access-do-not-echo".to_owned(),
        refresh_token: Some("xai-plan-refresh-do-not-echo".to_owned()),
        expires_at: Some(1),
    };
    let debug = format!("{file:?}");
    assert!(debug.contains("SecretString(<redacted>)"));
    assert!(!debug.contains("xai-plan-access"));
    assert!(!debug.contains("xai-plan-refresh"));
}

#[test]
fn incomplete_or_unknown_plan_file_fields_are_rejected() {
    for value in [
        serde_json::json!({
            "access_token": "token",
            "expires_at": null
        }),
        serde_json::json!({
            "access_token": "token",
            "refresh_token": null
        }),
        serde_json::json!({
            "access_token": "token",
            "refresh_token": null,
            "expires_at": null,
            "removed-field": true
        }),
    ] {
        assert!(serde_json::from_value::<PlanFile>(value).is_err());
    }
}

#[test]
fn device_code_fields_reject_unsafe_provider_values() {
    assert_eq!(
        super::sanitise_user_code("  ABCD-1234  ").as_deref(),
        Some("ABCD-1234")
    );
    for value in ["", "code with spaces", "code\nnext"] {
        assert!(super::sanitise_user_code(value).is_none());
    }
    assert!(super::sanitise_user_code(&"A".repeat(65)).is_none());

    assert_eq!(
        super::sanitise_verification_uri("  https://accounts.x.ai/connect  ").as_deref(),
        Some("https://accounts.x.ai/connect")
    );
    for value in [
        "http://accounts.x.ai/connect",
        "https://example.com/connect",
        "https://user@accounts.x.ai/connect",
        "https://accounts.x.ai/connect\nnext",
    ] {
        assert!(super::sanitise_verification_uri(value).is_none());
    }
    let long_query = format!("https://accounts.x.ai/connect?code={}", "a".repeat(513));
    assert!(super::sanitise_verification_uri(&long_query).is_none());
}

async fn response_from(raw: Vec<u8>) -> reqwest::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut written = 0;
        while written < raw.len() {
            stream.writable().await.expect("writable");
            match stream.try_write(&raw[written..]) {
                Ok(0) => panic!("response closed"),
                Ok(count) => written += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("response write: {error}"),
            }
        }
    });
    let response = reqwest::get(format!("http://{address}"))
        .await
        .expect("response");
    server.await.expect("server");
    response
}

#[tokio::test]
async fn oauth_response_bodies_enforce_the_byte_limit() {
    let declared = response_from(
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nabcde".to_vec(),
    )
    .await;
    assert!(super::bounded_text(declared, 4).await.is_none());

    let streamed = response_from(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nabcde\r\n0\r\n\r\n"
            .to_vec(),
    )
    .await;
    assert!(super::bounded_text(streamed, 4).await.is_none());
}
