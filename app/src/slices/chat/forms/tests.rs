use super::{ChatForm, CursorError, MAXIMUM_CURSOR, MAXIMUM_MESSAGE_BYTES, parse_cursor};

#[test]
fn rejects_empty_and_oversized_messages() {
    assert!(
        !ChatForm {
            message: "   ".to_owned()
        }
        .is_bounded()
    );
    assert!(
        !ChatForm {
            message: "a".repeat(MAXIMUM_MESSAGE_BYTES + 1)
        }
        .is_bounded()
    );
    assert!(
        ChatForm {
            message: "  hello  ".to_owned()
        }
        .is_bounded()
    );
}

#[test]
fn rejects_malformed_and_excessive_cursors() {
    assert_eq!(parse_cursor(""), Ok(0));
    assert_eq!(parse_cursor(" 12 "), Ok(12));
    assert_eq!(parse_cursor("0"), Ok(0));
    assert_eq!(
        parse_cursor(&MAXIMUM_CURSOR.to_string()),
        Ok(MAXIMUM_CURSOR)
    );
    assert_eq!(parse_cursor("-1"), Err(CursorError::Malformed));
    assert_eq!(parse_cursor("12a"), Err(CursorError::Malformed));
    assert_eq!(parse_cursor("1.5"), Err(CursorError::Malformed));
    assert_eq!(
        parse_cursor(&(MAXIMUM_CURSOR + 1).to_string()),
        Err(CursorError::Excessive)
    );
    assert_eq!(parse_cursor("10000000"), Err(CursorError::Excessive));
}
