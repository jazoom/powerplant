use super::{escape_plain, render};

#[test]
fn strips_script_from_markdown_html() {
    let html = render("<script>alert(1)</script>hello");
    assert!(!html.to_ascii_lowercase().contains("<script"), "{html}");
    assert!(!html.contains("alert(1)"), "{html}");
    assert!(html.contains("hello"), "{html}");
}

#[test]
fn strips_javascript_urls() {
    let html = render("[x](javascript:alert(1))");
    assert!(!html.to_ascii_lowercase().contains("javascript:"), "{html}");
}

#[test]
fn escapes_plain_user_text() {
    let html = escape_plain("<script>alert(1)</script>");
    assert!(!html.contains("<script>"), "{html}");
    assert!(html.contains("&lt;script&gt;"), "{html}");
}
