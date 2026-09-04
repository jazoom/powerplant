use askama::Template;

use crate::agents::{AgentRecord, MAXIMUM_GRANTS, ToolId};
use crate::projects::{ProjectRecord, desk_path, eligible_projects};
use crate::sandbox::OrphanSandbox;

use super::forms::AgentFormState;

pub(super) const CATALOGUE_TITLE: &str = "Agents | Power Plant";
pub(super) const NEW_TITLE: &str = "New agent | Power Plant";
pub(super) const CONFIG_TITLE: &str = "Configure agent | Power Plant";

pub(super) struct AgentListItem {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) projects: Vec<AgentProjectLink>,
}

pub(super) struct AgentProjectLink {
    pub(super) name: String,
    pub(super) href: String,
}

pub(super) struct GrantRow {
    pub(super) index: usize,
    pub(super) alias: String,
    pub(super) path: String,
    pub(super) access: String,
    pub(super) path_locked: bool,
    pub(super) can_remove: bool,
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
                    projects: eligible_projects(agent, projects)
                        .into_iter()
                        .map(|project| AgentProjectLink {
                            name: project.name.clone(),
                            href: desk_path(&project.id, &agent.id),
                        })
                        .collect(),
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
    pub(super) network: String,
    pub(super) network_domains: String,
    pub(super) tools: Vec<ToolRow>,
    pub(super) grants: Vec<GrantRow>,
    pub(super) can_add: bool,
    pub(super) error: &'static str,
    pub(super) agent_id: String,
    pub(super) revision: String,
    pub(super) show_delete: bool,
}

#[derive(Template)]
#[template(path = "agents/templates/form.html", block = "agent_form")]
pub(super) struct AgentFormContents<'a> {
    pub(super) action: &'a str,
    pub(super) submit: &'static str,
    pub(super) name: &'a str,
    pub(super) instructions: &'a str,
    pub(super) primary: &'a str,
    pub(super) network: &'a str,
    pub(super) network_domains: &'a str,
    pub(super) tools: &'a [ToolRow],
    pub(super) grants: &'a [GrantRow],
    pub(super) can_add: bool,
    pub(super) error: &'static str,
    pub(super) agent_id: &'a str,
    pub(super) revision: &'a str,
    pub(super) show_delete: bool,
}

impl AgentFormView {
    pub(super) fn create(state: AgentFormState, error: &'static str, project: &str) -> Self {
        Self::from_state(
            "New agent",
            &create_action(project),
            "Create agent",
            state,
            error,
            "",
            false,
        )
    }

    pub(super) fn create_for_project(
        mut state: AgentFormState,
        error: &'static str,
        project: &ProjectRecord,
    ) -> Self {
        state.assign_project_path(&project.host_path);
        let mut view = Self::from_state(
            "New agent",
            &create_action(&project.id.as_hex()),
            "Create agent",
            state,
            error,
            "",
            false,
        );
        if let Some(first) = view.grants.first_mut() {
            first.path_locked = true;
            first.can_remove = false;
        }
        view
    }

    pub(super) fn edit(record: &AgentRecord, state: AgentFormState, error: &'static str) -> Self {
        Self::from_state(
            "Configure agent",
            &format!("/agents/{}/configuration", record.id.as_hex()),
            "Save",
            state,
            error,
            &record.id.as_hex(),
            true,
        )
    }

    fn from_state(
        title: &'static str,
        action: &str,
        submit: &'static str,
        state: AgentFormState,
        error: &'static str,
        agent_id: &str,
        show_delete: bool,
    ) -> Self {
        let grant_count = state.directories.len();
        Self {
            title,
            action: action.to_owned(),
            submit,
            name: state.name,
            instructions: state.instructions,
            primary: state.primary,
            network: state.network,
            network_domains: state.network_domains,
            tools: ToolId::ALL
                .into_iter()
                .map(|tool| ToolRow {
                    name: tool.as_str(),
                    label: tool.label(),
                    checked: state.tools.contains(&tool),
                })
                .collect(),
            grants: state
                .directories
                .into_iter()
                .enumerate()
                .map(|(index, grant)| GrantRow {
                    index,
                    alias: grant.alias,
                    path: grant.path,
                    access: grant.access,
                    path_locked: false,
                    can_remove: grant_count > 1,
                })
                .collect(),
            can_add: grant_count < MAXIMUM_GRANTS,
            error,
            agent_id: agent_id.to_owned(),
            revision: state.revision,
            show_delete,
        }
    }

    pub(super) fn contents(&self) -> AgentFormContents<'_> {
        AgentFormContents {
            action: &self.action,
            submit: self.submit,
            name: &self.name,
            instructions: &self.instructions,
            primary: &self.primary,
            network: &self.network,
            network_domains: &self.network_domains,
            tools: &self.tools,
            grants: &self.grants,
            can_add: self.can_add,
            error: self.error,
            agent_id: &self.agent_id,
            revision: &self.revision,
            show_delete: self.show_delete,
        }
    }
}

fn create_action(project: &str) -> String {
    if project.is_empty() {
        "/agents".to_owned()
    } else {
        format!("/agents?project={project}")
    }
}
