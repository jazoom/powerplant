use super::{
    ChatForm, CursorError, MAXIMUM_CURSOR, MAXIMUM_MESSAGE_BYTES, ModelError, ModelForm,
    parse_cursor,
};
use crate::providers::{MAXIMUM_MODEL_BYTES, ProviderKind};

#[test]
fn rejects_empty_and_oversized_messages() {
    assert!(
        !ChatForm {
            message: "   ".to_owned(),
            mode: String::new(),
            workflow: String::new(),
        }
        .is_bounded()
    );
    assert!(
        !ChatForm {
            message: "a".repeat(MAXIMUM_MESSAGE_BYTES + 1),
            mode: String::new(),
            workflow: String::new(),
        }
        .is_bounded()
    );
    assert!(
        ChatForm {
            message: "  hello  ".to_owned(),
            mode: String::new(),
            workflow: String::new(),
        }
        .is_bounded()
    );
}

#[test]
fn accepts_quick_and_configured_modes() {
    use super::{DeskMode, DeskModeError};

    assert_eq!(
        ChatForm {
            message: "hello".to_owned(),
            mode: "quick".to_owned(),
            workflow: String::new(),
        }
        .mode(),
        Ok(DeskMode::Quick)
    );
    assert_eq!(
        ChatForm {
            message: "hello".to_owned(),
            mode: " configured ".to_owned(),
            workflow: String::new(),
        }
        .mode(),
        Ok(DeskMode::Configured)
    );
    assert_eq!(
        ChatForm {
            message: "hello".to_owned(),
            mode: String::new(),
            workflow: String::new(),
        }
        .mode(),
        Err(DeskModeError::Absent)
    );
    assert_eq!(
        ChatForm {
            message: "hello".to_owned(),
            mode: "chat".to_owned(),
            workflow: String::new(),
        }
        .mode(),
        Err(DeskModeError::Malformed)
    );
}

#[test]
fn workflow_selection_accepts_one_source_and_rejects_conflicts() {
    use super::WorkflowTokenError;

    let first = format!("{}:{}", "a".repeat(32), "b".repeat(64));
    let second = format!("{}:{}", "c".repeat(32), "d".repeat(64));
    let mut form = ChatForm {
        message: "hello".to_owned(),
        mode: "configured".to_owned(),
        workflow: String::new(),
    };
    assert!(form.workflow_selection(&first).is_ok());
    form.workflow = first;
    assert_eq!(
        form.workflow_selection(&second),
        Err(WorkflowTokenError::Malformed)
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
        favourite: None,
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
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
        favourite: None,
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert_eq!(form.validate(|_| true), Err(ModelError::Provider));
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: String::new(),
        favourite: None,
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert_eq!(form.validate(|_| false), Err(ModelError::Provider));
}

#[test]
fn favourite_toggle_requires_a_present_model() {
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: "  ".to_owned(),
        favourite: Some("  ".to_owned()),
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert!(form.wants_favourite_toggle());
    assert_eq!(form.validate_favourite(|_| true), Err(ModelError::Model));
}

#[test]
fn favourite_toggle_keeps_the_model_id_verbatim() {
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: "grok-4.6".to_owned(),
        favourite: Some("  grok-4-mini  ".to_owned()),
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert!(form.wants_favourite_toggle());
    assert_eq!(
        form.validate_favourite(|kind| kind == ProviderKind::Xai),
        Ok((ProviderKind::Xai, "grok-4-mini".to_owned()))
    );
}

#[test]
fn model_form_rejects_an_oversized_or_control_model() {
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: "a".repeat(MAXIMUM_MODEL_BYTES + 1),
        favourite: None,
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert_eq!(form.validate(|_| true), Err(ModelError::Model));
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: "grok-\u{0000}".to_owned(),
        favourite: None,
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert_eq!(form.validate(|_| true), Err(ModelError::Model));
}
