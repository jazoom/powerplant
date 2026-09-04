use super::{
    MAXIMUM_MODEL_REPLY_BYTES, MAXIMUM_REPLY_BYTES, MAXIMUM_TOOL_PREVIEW_BYTES, append_model_piece,
    append_visible_tool_trace,
};

#[test]
fn a_large_tool_result_does_not_consume_the_model_reply_limit() {
    let trace = crate::tools::render_trace_preview(
        "read `/project/large.txt`",
        &"x".repeat(crate::tools::MAXIMUM_TOOL_BYTES),
        MAXIMUM_TOOL_PREVIEW_BYTES,
    );
    let mut reply = String::new();
    let mut visible_tool_bytes = 0;
    append_visible_tool_trace(&mut reply, &trace, &mut visible_tool_bytes);

    let mut model_reply_bytes = 0;
    assert!(!append_model_piece(
        &mut reply,
        &"a".repeat(MAXIMUM_MODEL_REPLY_BYTES),
        &mut model_reply_bytes,
    ));
    assert!(reply.contains("[output truncated]"));
    assert!(reply.len() <= MAXIMUM_REPLY_BYTES);
}

#[test]
fn model_reply_overflow_remains_an_error() {
    let mut reply = String::new();
    let mut model_reply_bytes = 0;
    assert!(append_model_piece(
        &mut reply,
        &"a".repeat(MAXIMUM_MODEL_REPLY_BYTES + 1),
        &mut model_reply_bytes,
    ));
    assert_eq!(reply.len(), MAXIMUM_MODEL_REPLY_BYTES);
}
