use super::AgentForm;
use crate::agents::{AccessMode, AgentError, ToolId};
use std::collections::HashMap;

#[test]
fn agent_form_reads_tools_and_grants() {
    let mut extra = HashMap::new();
    extra.insert("tool_list".to_owned(), "on".to_owned());
    extra.insert("tool_read".to_owned(), "on".to_owned());
    extra.insert("alias_0".to_owned(), "project".to_owned());
    extra.insert("path_0".to_owned(), "/tmp/app".to_owned());
    extra.insert("access_0".to_owned(), "read-write".to_owned());
    extra.insert("alias_1".to_owned(), String::new());
    extra.insert("path_1".to_owned(), String::new());
    let form = AgentForm {
        name: "Maintainer".to_owned(),
        instructions: String::new(),
        primary: "project".to_owned(),
        revision: String::new(),
        extra,
    };
    let draft = form.draft().expect("draft");
    assert_eq!(draft.tools, [ToolId::List, ToolId::Read]);
    assert_eq!(draft.directories.len(), 1);
    assert_eq!(draft.directories[0].access, AccessMode::ReadWrite);
}

#[test]
fn oversized_name_is_rejected() {
    let form = AgentForm {
        name: "a".repeat(crate::agents::MAXIMUM_NAME_BYTES + 1),
        instructions: String::new(),
        primary: "project".to_owned(),
        revision: String::new(),
        extra: HashMap::new(),
    };
    assert_eq!(form.draft().err(), Some(AgentError::Name));
}

fn revision_form(revision: &str) -> AgentForm {
    AgentForm {
        name: String::new(),
        instructions: String::new(),
        primary: String::new(),
        revision: revision.to_owned(),
        extra: HashMap::new(),
    }
}

#[test]
fn revision_parser_accepts_positive_decimals() {
    assert_eq!(revision_form("1").revision(), Ok(Some(1)));
    assert_eq!(
        revision_form(&u32::MAX.to_string()).revision(),
        Ok(Some(u32::MAX))
    );
    assert_eq!(revision_form("").revision(), Ok(None));
    assert_eq!(revision_form(" 2 ").revision(), Ok(Some(2)));
}

#[test]
fn revision_parser_rejects_malformed_and_excessive_values() {
    for value in ["0", "01", "1a", "4294967296", "1.0", "-1"] {
        assert_eq!(
            revision_form(value).revision(),
            Err(super::REVISION_MESSAGE),
            "{value}"
        );
    }
}
