use super::{ModelError, ModelForm};
use crate::providers::{MAXIMUM_MODEL_BYTES, ProviderKind};

#[test]
fn model_form_accepts_a_stored_provider_and_blank_model() {
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: "  ".to_owned(),
        favourite: None,
        thinking: "default".to_owned(),
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert_eq!(
        form.validate(|kind| kind == ProviderKind::Xai),
        Ok((
            ProviderKind::Xai,
            ProviderKind::Xai.default_model().to_owned(),
            None,
        ))
    );
}

#[test]
fn model_form_rejects_unknown_or_unstored_providers() {
    let form = ModelForm {
        provider: "openai".to_owned(),
        model: String::new(),
        favourite: None,
        thinking: "default".to_owned(),
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert_eq!(form.validate(|_| true), Err(ModelError::Provider));
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: String::new(),
        favourite: None,
        thinking: "default".to_owned(),
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
        thinking: "default".to_owned(),
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
        thinking: "default".to_owned(),
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
        thinking: "default".to_owned(),
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert_eq!(form.validate(|_| true), Err(ModelError::Model));
    let form = ModelForm {
        provider: "xai".to_owned(),
        model: "grok-\u{0000}".to_owned(),
        favourite: None,
        thinking: "default".to_owned(),
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert_eq!(form.validate(|_| true), Err(ModelError::Model));
}

#[test]
fn model_form_accepts_a_bounded_dynamic_thinking_effort() {
    let form = ModelForm {
        provider: "deepseek".to_owned(),
        model: "deepseek-v4-flash".to_owned(),
        favourite: None,
        thinking: "low".to_owned(),
        provider_model_synced: false,
        project: String::new(),
        agent: String::new(),
    };
    assert_eq!(
        form.validate(|_| true),
        Ok((
            ProviderKind::Deepseek,
            "deepseek-v4-flash".to_owned(),
            Some(crate::providers::ThinkingEffort::new("low".to_owned()).unwrap()),
        ))
    );
}
