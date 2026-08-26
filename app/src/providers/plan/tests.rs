#[test]
fn device_prompts_reject_unsafe_provider_values() {
    assert_eq!(
        super::sanitise_user_code("  ABCD-1234  ").as_deref(),
        Some("ABCD-1234")
    );
    for value in ["", "code with spaces", "code\nnext"] {
        assert!(super::sanitise_user_code(value).is_none());
    }
    assert!(super::sanitise_user_code(&"A".repeat(65)).is_none());

    assert_eq!(
        super::sanitise_https_uri("  https://example.com/device  ").as_deref(),
        Some("https://example.com/device")
    );
    for value in [
        "http://example.com/device",
        "javascript:alert(1)",
        "https://user@example.com/device",
        "https://example.com/device\nnext",
    ] {
        assert!(super::sanitise_https_uri(value).is_none());
    }
    assert!(
        super::sanitise_https_uri(&format!("https://example.com/{}", "a".repeat(2_048))).is_none()
    );
}
