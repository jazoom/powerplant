use super::{AgentFormState, FormError, FormIntent};
use crate::agents::{AccessMode, AgentError, NetworkAccess, ToolId};

fn pair(key: &str, value: &str) -> (String, String) {
    (key.to_owned(), value.to_owned())
}

fn valid_pairs() -> Vec<(String, String)> {
    vec![
        pair("intent", "save"),
        pair("name", "Maintainer"),
        pair("instructions", "Keep going"),
        pair("network", "none"),
        pair("network_domains", ""),
        pair("primary", "project"),
        pair("tool_list", "on"),
        pair("tool_read", "on"),
        pair("alias_0", "project"),
        pair("path_0", "/tmp/app"),
        pair("access_0", "read-write"),
    ]
}

#[test]
fn agent_form_reads_tools_and_grants() {
    let (form, intent) = AgentFormState::parse(valid_pairs()).expect("parse");
    assert_eq!(intent, FormIntent::Save);
    let draft = form.draft().expect("draft");
    assert_eq!(draft.tools, [ToolId::List, ToolId::Read]);
    assert_eq!(draft.network, NetworkAccess::None);
    assert_eq!(draft.directories.len(), 1);
    assert_eq!(draft.directories[0].access, AccessMode::ReadWrite);
}

#[test]
fn restricted_network_domains_are_parsed_and_normalised() {
    let mut pairs = valid_pairs();
    pairs[3] = pair("network", "restricted");
    pairs[4] = pair("network_domains", "Registry.NPMJS.org\ngithub.com");
    let (form, _) = AgentFormState::parse(pairs).expect("parse");
    assert_eq!(
        form.draft().expect("draft").network,
        NetworkAccess::Restricted(vec![
            "registry.npmjs.org".to_owned(),
            "github.com".to_owned(),
        ])
    );
}

#[test]
fn invalid_restricted_network_domains_are_rejected() {
    let mut pairs = valid_pairs();
    pairs[3] = pair("network", "restricted");
    pairs[4] = pair("network_domains", "https://example.com/path");
    let (form, _) = AgentFormState::parse(pairs).expect("parse");
    assert_eq!(form.draft().err(), Some(AgentError::Network));
}

#[test]
fn oversized_name_is_rejected() {
    let mut pairs = valid_pairs();
    pairs[1] = pair("name", &"a".repeat(crate::agents::MAXIMUM_NAME_BYTES + 1));
    let (form, _) = AgentFormState::parse(pairs).expect("parse");
    assert_eq!(form.draft().err(), Some(AgentError::Name));
}

#[test]
fn unknown_intents_are_rejected() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "explode");
    assert_eq!(AgentFormState::parse(pairs).err(), Some(FormError::Intent));
}

#[test]
fn missing_intent_is_rejected() {
    let mut pairs = valid_pairs();
    pairs.remove(0);
    assert_eq!(AgentFormState::parse(pairs).err(), Some(FormError::Intent));
}

#[test]
fn unknown_fields_are_rejected() {
    let mut pairs = valid_pairs();
    pairs.push(pair("shell", "rm -rf /"));
    assert_eq!(
        AgentFormState::parse(pairs).err(),
        Some(FormError::UnknownField)
    );
}

#[test]
fn duplicate_fields_are_rejected() {
    let mut pairs = valid_pairs();
    pairs.push(pair("name", "Other"));
    assert_eq!(
        AgentFormState::parse(pairs).err(),
        Some(FormError::DuplicateField)
    );
}

#[test]
fn sparse_directory_indices_are_rejected() {
    let mut pairs = valid_pairs();
    pairs.push(pair("alias_2", "docs"));
    pairs.push(pair("path_2", "/tmp/docs"));
    pairs.push(pair("access_2", "read-only"));
    assert_eq!(AgentFormState::parse(pairs).err(), Some(FormError::Sparse));
}

#[test]
fn malformed_indices_are_rejected() {
    let mut pairs = valid_pairs();
    pairs.push(pair("alias_01", "padded"));
    assert_eq!(AgentFormState::parse(pairs).err(), Some(FormError::Index));
}

#[test]
fn excessive_directory_indices_are_rejected() {
    let mut pairs = valid_pairs();
    pairs.push(pair(
        &format!("alias_{}", crate::agents::MAXIMUM_GRANTS),
        "extra",
    ));
    assert_eq!(
        AgentFormState::parse(pairs).err(),
        Some(FormError::Excessive)
    );
}

#[test]
fn add_directory_appends_a_blank_row_without_path_validation() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "add-directory");
    let (mut form, intent) = AgentFormState::parse(pairs).expect("parse");
    assert_eq!(intent, FormIntent::AddDirectory);
    form.apply(intent).expect("apply");
    assert_eq!(form.directories.len(), 2);
    assert_eq!(form.directories[0].alias, "project");
    assert_eq!(form.directories[0].path, "/tmp/app");
    assert_eq!(form.directories[1].alias, "");
    assert_eq!(form.directories[1].path, "");
    assert_eq!(form.directories[1].access, "read-write");
    assert_eq!(form.name, "Maintainer");
    assert_eq!(form.instructions, "Keep going");
    assert_eq!(form.network, "none");
    assert!(form.network_domains.is_empty());
    assert_eq!(form.primary, "project");
    assert_eq!(form.tools, [ToolId::List, ToolId::Read]);
}

#[test]
fn add_directory_is_rejected_at_the_grant_limit() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "add-directory");
    for index in 1..crate::agents::MAXIMUM_GRANTS {
        pairs.push(pair(&format!("alias_{index}"), "docs"));
        pairs.push(pair(&format!("path_{index}"), "/tmp/docs"));
        pairs.push(pair(&format!("access_{index}"), "read-only"));
    }
    let (mut form, intent) = AgentFormState::parse(pairs).expect("parse");
    assert_eq!(form.directories.len(), crate::agents::MAXIMUM_GRANTS);
    assert_eq!(form.apply(intent).err(), Some(FormError::Excessive));
}

#[test]
fn remove_directory_compacts_later_rows_and_keeps_state() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "remove-directory:0");
    pairs.push(pair("alias_1", "docs"));
    pairs.push(pair("path_1", "/tmp/docs"));
    pairs.push(pair("access_1", "read-only"));
    let (mut form, intent) = AgentFormState::parse(pairs).expect("parse");
    form.apply(intent).expect("apply");
    assert_eq!(form.directories.len(), 1);
    assert_eq!(form.directories[0].alias, "docs");
    assert_eq!(form.directories[0].path, "/tmp/docs");
    assert_eq!(form.directories[0].access, "read-only");
    assert_eq!(form.primary, "docs");
    assert_eq!(form.name, "Maintainer");
}

#[test]
fn remove_directory_keeps_primary_when_another_row_owns_it() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "remove-directory:1");
    pairs.push(pair("alias_1", "docs"));
    pairs.push(pair("path_1", "/tmp/docs"));
    pairs.push(pair("access_1", "read-only"));
    let (mut form, intent) = AgentFormState::parse(pairs).expect("parse");
    form.apply(intent).expect("apply");
    assert_eq!(form.directories.len(), 1);
    assert_eq!(form.primary, "project");
}

#[test]
fn remove_directory_rejects_the_last_row() {
    let mut pairs = valid_pairs();
    pairs[0] = pair("intent", "remove-directory:0");
    let (mut form, intent) = AgentFormState::parse(pairs).expect("parse");
    assert_eq!(form.apply(intent).err(), Some(FormError::Index));
    assert_eq!(form.directories.len(), 1);
}

fn revision_form(revision: &str) -> AgentFormState {
    let mut form = AgentFormState::blank();
    form.revision = revision.to_owned();
    form
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
