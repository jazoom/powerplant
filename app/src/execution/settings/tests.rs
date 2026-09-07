use crate::{
    agents::ToolId,
    providers::{ModelSelection, ProviderKind},
};

use super::ExecutionSettings;

fn model() -> ModelSelection {
    ModelSelection::new(ProviderKind::Xai, "test-model".to_owned(), None).unwrap()
}

#[test]
fn settings_reject_duplicate_tools_and_control_characters() {
    assert!(
        ExecutionSettings::new(model(), String::new(), vec![ToolId::Read, ToolId::Read],).is_none()
    );
    assert!(ExecutionSettings::new(model(), "bad\0text".to_owned(), Vec::new()).is_none());
}
