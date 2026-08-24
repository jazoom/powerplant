use std::time::{Duration, Instant};

use super::{
    MAXIMUM_LISTED_MODELS, MAXIMUM_MODEL_BYTES, ProviderError, ProviderKind, SecretString,
    classify_failure_status, classify_verify_status,
    rig::{VERIFY_TIMEOUT, models_at, parse_model_list, verify_at},
};

#[test]
fn parses_known_providers() {
    assert_eq!(ProviderKind::parse("xai"), Some(ProviderKind::Xai));
    assert_eq!(
        ProviderKind::parse("openai-codex"),
        Some(ProviderKind::OpenaiCodex)
    );
    assert_eq!(
        ProviderKind::parse("synthetic"),
        Some(ProviderKind::Synthetic)
    );
    assert_eq!(ProviderKind::parse("openai"), None);
}

#[test]
fn secret_debug_is_redacted() {
    let secret = SecretString::new("sk-secret-value".to_owned());
    let debug = format!("{secret:?}");
    assert_eq!(debug, "SecretString(<redacted>)");
    assert!(!debug.contains("sk-secret"));
}

#[test]
fn secret_new_trims_the_key() {
    let secret = SecretString::new("  sk-secret-value\n".to_owned());
    assert_eq!(secret.expose(), "sk-secret-value");
}

#[test]
fn verify_status_treats_auth_failures_as_rejection() {
    assert_eq!(
        classify_verify_status(401, None),
        Err(ProviderError::Rejected)
    );
    assert_eq!(
        classify_verify_status(403, None),
        Err(ProviderError::Rejected)
    );
}

#[test]
fn verify_status_treats_rate_limits_as_rate_limited() {
    assert_eq!(
        classify_verify_status(429, None),
        Err(ProviderError::RateLimited { retry_after: None })
    );
    assert_eq!(
        classify_verify_status(429, Some("15")),
        Err(ProviderError::RateLimited {
            retry_after: hypergraft::RetryAfter::seconds(15),
        })
    );
    assert_eq!(
        classify_verify_status(429, Some(" 8 ")),
        Err(ProviderError::RateLimited {
            retry_after: hypergraft::RetryAfter::seconds(8),
        })
    );
    assert_eq!(
        classify_verify_status(429, Some("0")),
        Err(ProviderError::RateLimited { retry_after: None })
    );
    assert_eq!(
        classify_verify_status(429, Some("Mon, 01 Jan 2020 00:00:00 GMT")),
        Err(ProviderError::RateLimited { retry_after: None })
    );
}

#[test]
fn verify_status_treats_transport_and_server_errors_as_unavailability() {
    assert_eq!(
        classify_verify_status(500, None),
        Err(ProviderError::Unreachable)
    );
    assert_eq!(
        classify_verify_status(502, None),
        Err(ProviderError::Unreachable)
    );
    assert_eq!(
        classify_verify_status(503, None),
        Err(ProviderError::Unreachable)
    );
    assert_eq!(
        classify_verify_status(408, None),
        Err(ProviderError::Unreachable)
    );
}

#[test]
fn verify_status_accepts_success_and_authenticated_request_errors() {
    assert_eq!(classify_verify_status(200, None), Ok(()));
    assert_eq!(classify_verify_status(204, None), Ok(()));
    assert_eq!(classify_verify_status(400, None), Ok(()));
    assert_eq!(classify_verify_status(422, None), Ok(()));
}

#[test]
fn completion_failures_use_the_same_status_families() {
    assert_eq!(classify_failure_status(401, None), ProviderError::Rejected);
    assert_eq!(classify_failure_status(403, None), ProviderError::Rejected);
    assert_eq!(
        classify_failure_status(429, Some("20")),
        ProviderError::RateLimited {
            retry_after: hypergraft::RetryAfter::seconds(20),
        }
    );
    assert_eq!(
        classify_failure_status(500, None),
        ProviderError::Unreachable
    );
}

#[test]
fn each_provider_outcome_maps_to_one_patch_status() {
    assert_eq!(
        ProviderError::Rejected.patch_status(),
        hypergraft::PatchStatus::Unauthorized
    );
    assert_eq!(
        ProviderError::Unreachable.patch_status(),
        hypergraft::PatchStatus::UnprocessableEntity
    );
    assert_eq!(
        ProviderError::EmptyReply.patch_status(),
        hypergraft::PatchStatus::UnprocessableEntity
    );
    assert_eq!(
        ProviderError::ReplyTooLong.patch_status(),
        hypergraft::PatchStatus::UnprocessableEntity
    );
    assert_eq!(
        ProviderError::RateLimited {
            retry_after: hypergraft::RetryAfter::seconds(12),
        }
        .patch_status(),
        hypergraft::PatchStatus::TooManyRequests(hypergraft::RetryAfter::seconds(12).unwrap())
    );
    assert_eq!(
        ProviderError::RateLimited { retry_after: None }.patch_status(),
        hypergraft::PatchStatus::TooManyRequests(hypergraft::RetryAfter::seconds(1).unwrap())
    );
}

#[test]
fn recovery_copy_does_not_blame_a_valid_key_for_an_outage() {
    assert!(
        ProviderError::Unreachable
            .message()
            .contains("could not be reached")
    );
    assert!(!ProviderError::Unreachable.message().contains("key"));
    assert!(
        ProviderError::Rejected
            .message()
            .contains("key was rejected")
    );
    assert!(
        ProviderError::RateLimited { retry_after: None }
            .message()
            .contains("rate-limited")
    );
}

#[test]
fn the_model_list_parser_bounds_and_dedupes() {
    let body = serde_json::json!({
        "data": [
            {"id": "grok-4.6"},
            {"id": "grok-4.6"},
            {"id": "  grok-4-mini  "},
            {"id": ""},
            {"id": "bad\u{0000}id"},
            {"not-id": 3},
            {"id": "a".repeat(MAXIMUM_MODEL_BYTES + 1)}
        ]
    })
    .to_string();
    assert_eq!(
        parse_model_list(body.as_bytes()),
        vec!["grok-4-mini".to_owned(), "grok-4.6".to_owned()]
    );
    assert_eq!(parse_model_list(b"{not-json"), Vec::<String>::new());

    let many = serde_json::json!({
        "data": (0..(MAXIMUM_LISTED_MODELS + 10))
            .map(|index| serde_json::json!({"id": format!("model-{index}")}))
            .collect::<Vec<_>>()
    })
    .to_string();
    assert_eq!(
        parse_model_list(many.as_bytes()).len(),
        MAXIMUM_LISTED_MODELS
    );
}

#[tokio::test]
async fn stalled_verification_ends_within_the_configured_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let Ok((_stream, _)) = listener.accept().await else {
            return;
        };
        std::future::pending::<()>().await;
    });

    let timeout = Duration::from_millis(80);
    let started = Instant::now();
    let result = verify_at(&format!("http://{addr}"), "sk-test", timeout).await;
    let elapsed = started.elapsed();

    assert_eq!(result, Err(ProviderError::Unreachable));
    assert!(elapsed >= timeout, "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
    assert_eq!(VERIFY_TIMEOUT, Duration::from_secs(10));
}

#[tokio::test]
async fn a_stalled_model_body_ends_within_the_configured_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        if stream.writable().await.is_err() {
            return;
        }
        let _ = stream.try_write(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{",
        );
        std::future::pending::<()>().await;
    });

    let timeout = Duration::from_millis(80);
    let started = Instant::now();
    let result = models_at(&format!("http://{addr}"), "sk-test", timeout).await;
    let elapsed = started.elapsed();

    assert_eq!(result, Err(ProviderError::Unreachable));
    assert!(elapsed >= timeout, "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
}

#[tokio::test]
async fn a_successful_models_response_returns_synthetic_model_ids() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let body = serde_json::json!({
            "object": "list",
            "data": [
                {"id": "syn:large:text", "object": "model"},
                {"id": "hf:moonshotai/Kimi-K3", "object": "model"}
            ]
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut written = 0;
        while written < response.len() {
            if stream.writable().await.is_err() {
                return;
            }
            match stream.try_write(&response.as_bytes()[written..]) {
                Ok(0) => return,
                Ok(count) => written += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return,
            }
        }
    });

    let models = models_at(&format!("http://{addr}"), "sk-test", Duration::from_secs(2))
        .await
        .expect("models");

    assert_eq!(models, ["hf:moonshotai/Kimi-K3", "syn:large:text"]);
}

#[tokio::test]
async fn verify_preserves_retry_after_and_drops_the_provider_body() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let response = b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 12\r\nContent-Length: 32\r\nConnection: close\r\n\r\nsecret-provider-body-do-not-copy";
        let mut written = 0;
        while written < response.len() {
            if stream.writable().await.is_err() {
                return;
            }
            match stream.try_write(&response[written..]) {
                Ok(0) => return,
                Ok(count) => written += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return,
            }
        }
    });

    let result = verify_at(&format!("http://{addr}"), "sk-test", Duration::from_secs(2)).await;
    assert_eq!(
        result,
        Err(ProviderError::RateLimited {
            retry_after: hypergraft::RetryAfter::seconds(12),
        })
    );
    let display = result.expect_err("rate limit");
    assert!(!display.to_string().contains("secret-provider-body"));
}
