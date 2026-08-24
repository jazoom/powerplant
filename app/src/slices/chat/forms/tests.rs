use super::{ChatForm, MAXIMUM_MESSAGE_BYTES};

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
