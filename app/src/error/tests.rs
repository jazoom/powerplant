use super::*;

use axum::http::{StatusCode, header};

#[derive(Debug)]
struct SecretSource;

impl fmt::Display for SecretSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("api_key=do-not-log")
    }
}

impl Error for SecretSource {}

#[test]
fn debug_logs_and_browser_response_do_not_expose_the_source_message() {
    let tracing = tracing_test_guard();
    let error = AppError::new("call provider", SecretSource);
    let debug = format!("{error:?}");
    assert!(debug.contains("call provider"));
    assert!(debug.contains("SecretSource"));
    assert!(!debug.contains("do-not-log"));

    let response = error.into_response();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let output = tracing.output();
    assert!(output.contains("operation=\"call provider\""), "{output}");
    assert!(
        output.contains("source=\"circus::error::tests::SecretSource\""),
        "{output}"
    );
    assert!(!output.contains("do-not-log"), "{output}");
}
