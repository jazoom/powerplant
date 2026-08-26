use super::{MAXIMUM_TOOL_BYTES, guest_path, mark_truncated, redact, render_trace};

#[test]
fn guest_path_stays_inside_the_project() {
    assert_eq!(guest_path("").expect("default"), "/project");
    assert_eq!(
        guest_path("src/main.rs").expect("relative"),
        "/project/src/main.rs"
    );
    assert_eq!(
        guest_path("/project/src/../lib.rs").expect("dotdot"),
        "/project/lib.rs"
    );
    assert_eq!(guest_path(".").expect("dot"), "/project");
}

#[test]
fn guest_path_rejects_escape_and_control() {
    assert_eq!(guest_path(".."), Err("Stay inside the project directory."));
    assert_eq!(
        guest_path("/etc/passwd"),
        Err("Stay inside the project directory.")
    );
    assert_eq!(
        guest_path("/project/../../secret"),
        Err("Stay inside the project directory.")
    );
    assert_eq!(guest_path("/tmp/\u{0000}x"), Err("That path is not valid."));
}

#[test]
fn redact_removes_the_vault_secret() {
    assert_eq!(
        redact("token sk-secret in output", Some("sk-secret")),
        "token [redacted] in output"
    );
    assert_eq!(redact("plain", None), "plain");
    assert_eq!(redact("plain", Some("")), "plain");
}

#[test]
fn a_tool_trace_keeps_untrusted_content_inside_html_text() {
    let trace = render_trace("run **false**", "~~~\n[forged](https://example.com)");
    assert!(trace.contains("~~~\n[forged](https://example.com)"));
    let html = crate::markdown::render(&trace);
    assert_eq!(html.matches("<strong>").count(), 1, "{html}");
    assert!(!html.contains("<a "), "{html}");
    assert!(html.contains("<pre><code>"), "{html}");
}

#[test]
fn truncated_tool_output_carries_a_bounded_marker() {
    let mut output = "x".repeat(MAXIMUM_TOOL_BYTES);
    mark_truncated(&mut output);
    assert_eq!(output.len(), MAXIMUM_TOOL_BYTES);
    assert!(output.ends_with("[output truncated]"));
}
