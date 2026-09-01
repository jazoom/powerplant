use askama::Template;

use crate::agents::{AccessMode, AgentRecord, ToolId};
use crate::projects::{ProjectRecord, unique_desk_path};
use crate::sandbox::OrphanSandbox;

pub(super) const CATALOGUE_TITLE: &str = "Agents | Power Plant";
pub(super) const NEW_TITLE: &str = "New agent | Power Plant";
pub(super) const CONFIG_TITLE: &str = "Configure agent | Power Plant";

pub(super) struct AgentListItem {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) desk_href: String,
}

pub(super) struct GrantRow {
    pub(super) index: usize,
    pub(super) alias: String,
    pub(super) path: String,
    pub(super) read_write: bool,
}

pub(super) struct ToolRow {
    pub(super) name: &'static str,
    pub(super) label: &'static str,
    pub(super) checked: bool,
}

#[derive(Template)]
#[template(path = "agents/templates/catalogue.html")]
pub(super) struct CatalogueView {
    pub(super) agents: Vec<AgentListItem>,
    pub(super) orphans: Vec<OrphanSandbox>,
    pub(super) error: &'static str,
}

impl CatalogueView {
    pub(super) fn from_parts(
        agents: &[AgentRecord],
        projects: &[ProjectRecord],
        orphans: Vec<OrphanSandbox>,
        error: &'static str,
    ) -> Self {
        Self {
            agents: agents
                .iter()
                .map(|agent| AgentListItem {
                    id: agent.id.as_hex(),
                    name: agent.name.clone(),
                    desk_href: unique_desk_path(agent, projects).unwrap_or_default(),
                })
                .collect(),
            orphans,
            error,
        }
    }
}

#[derive(Template)]
#[template(path = "agents/templates/form.html")]
pub(super) struct AgentFormView {
    pub(super) title: &'static str,
    pub(super) action: String,
    pub(super) submit: &'static str,
    pub(super) name: String,
    pub(super) instructions: String,
    pub(super) primary: String,
    pub(super) tools: Vec<ToolRow>,
    pub(super) grants: Vec<GrantRow>,
    pub(super) error: &'static str,
    pub(super) agent_id: String,
    pub(super) revision: String,
    pub(super) show_delete: bool,
}

impl AgentFormView {
    pub(super) fn create(error: &'static str) -> Self {
        Self::from_record(None, "/agents", "Create agent", error)
    }

    pub(super) fn edit(record: &AgentRecord, error: &'static str) -> Self {
        Self::from_record(
            Some(record),
            &format!("/agents/{}/configuration", record.id.as_hex()),
            "Save",
            error,
        )
    }

    fn from_record(
        record: Option<&AgentRecord>,
        action: &str,
        submit: &'static str,
        error: &'static str,
    ) -> Self {
        let selected: &[ToolId] = match record {
            Some(record) => record.tools.as_slice(),
            None => &ToolId::ALL,
        };
        let mut grants = record
            .map(|record| {
                record
                    .directories
                    .iter()
                    .enumerate()
                    .map(|(index, grant)| GrantRow {
                        index,
                        alias: grant.alias.clone(),
                        path: grant.host_path.to_string_lossy().into_owned(),
                        read_write: grant.access == AccessMode::ReadWrite,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                vec![GrantRow {
                    index: 0,
                    alias: "project".to_owned(),
                    path: String::new(),
                    read_write: true,
                }]
            });
        while grants.len() < crate::agents::MAXIMUM_GRANTS {
            let index = grants.len();
            grants.push(GrantRow {
                index,
                alias: String::new(),
                path: String::new(),
                read_write: true,
            });
        }
        Self {
            title: if record.is_some() {
                "Configure agent"
            } else {
                "New agent"
            },
            action: action.to_owned(),
            submit,
            name: record.map(|record| record.name.clone()).unwrap_or_default(),
            instructions: record
                .map(|record| record.instructions.clone())
                .unwrap_or_default(),
            primary: record
                .map(|record| record.primary_directory.clone())
                .unwrap_or_else(|| "project".to_owned()),
            tools: ToolId::ALL
                .into_iter()
                .map(|tool| ToolRow {
                    name: tool.as_str(),
                    label: tool.label(),
                    checked: selected.contains(&tool),
                })
                .collect(),
            grants,
            error,
            agent_id: record.map(|record| record.id.as_hex()).unwrap_or_default(),
            revision: record
                .map(|record| record.revision.to_string())
                .unwrap_or_default(),
            show_delete: record.is_some(),
        }
    }
}
