use super::{
    ChatForm, CursorError, MAXIMUM_CURSOR, MAXIMUM_MESSAGE_BYTES, ModelError, ModelForm,
    parse_cursor,
};
use crate::providers::{MAXIMUM_MODEL_BYTES, ProviderKind};

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

#[test]
fn model_form_accepts_a_stored_provider_and_blank_model() {
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: "  ".to_owned(),
    };
    assert_eq!(
        form.validate(|kind| kind == ProviderKind::Xai),
        Ok((
            ProviderKind::Xai,
            ProviderKind::Xai.default_model().to_owned()
        ))
    );
}

#[test]
fn model_form_rejects_unknown_or_unstored_providers() {
    let form = ModelForm {
        provider: "openai".to_owned(),
        model: String::new(),
    };
    assert_eq!(form.validate(|_| true), Err(ModelError::Provider));
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: String::new(),
    };
    assert_eq!(form.validate(|_| false), Err(ModelError::Provider));
}

#[test]
fn model_form_rejects_an_oversized_or_control_model() {
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: "a".repeat(MAXIMUM_MODEL_BYTES + 1),
    };
    assert_eq!(form.validate(|_| true), Err(ModelError::Model));
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: "grok-\u{0000}".to_owned(),
    };
    assert_eq!(form.validate(|_| true), Err(ModelError::Model));
}
