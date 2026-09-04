use askama::Template;

use crate::agents::AgentRecord;
use crate::projects::{ProjectRecord, desk_path};

pub(super) const INDEX_TITLE: &str = "Projects | Power Plant";
pub(super) const NEW_TITLE: &str = "New project | Power Plant";
pub(super) const CONFIG_TITLE: &str = "Rename project | Power Plant";

pub(super) struct CatalogueItem {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) available: bool,
}

#[derive(Template)]
#[template(path = "projects/templates/index.html")]
pub(super) struct CatalogueView {
    pub(super) projects: Vec<CatalogueItem>,
}

impl CatalogueView {
    pub(super) fn from_records(records: &[ProjectRecord]) -> Self {
        Self {
            projects: records
                .iter()
                .map(|record| CatalogueItem {
                    id: record.id.as_hex(),
                    name: record.name.clone(),
                    path: record.host_path.to_string_lossy().into_owned(),
                    available: record.host_path_is_available(),
                })
                .collect(),
        }
    }
}

pub(super) struct EligibleAgentLink {
    pub(super) name: String,
    pub(super) href: String,
}

pub(super) struct GrantCandidate {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) revision: String,
}

#[derive(Template)]
#[template(path = "projects/templates/detail.html")]
pub(super) struct DetailView {
    pub(super) document_title: String,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) available: bool,
    pub(super) agents: Vec<EligibleAgentLink>,
    pub(super) starter_action: String,
    pub(super) starter_href: String,
    pub(super) grant_action: String,
    pub(super) grant_candidates: Vec<GrantCandidate>,
    pub(super) grant_alias: String,
    pub(super) grant_access: String,
    pub(super) error: &'static str,
}

impl DetailView {
    pub(super) fn from_record(
        record: &ProjectRecord,
        eligible: &[AgentRecord],
        catalogue: &[AgentRecord],
    ) -> Self {
        Self::with_grant(record, eligible, catalogue, "project", "read-write", "")
    }

    pub(super) fn with_grant(
        record: &ProjectRecord,
        eligible: &[AgentRecord],
        catalogue: &[AgentRecord],
        grant_alias: &str,
        grant_access: &str,
        error: &'static str,
    ) -> Self {
        Self {
            document_title: format!("{} | Power Plant", record.name),
            id: record.id.as_hex(),
            name: record.name.clone(),
            path: record.host_path.to_string_lossy().into_owned(),
            available: record.host_path_is_available(),
            agents: eligible
                .iter()
                .map(|agent| EligibleAgentLink {
                    name: agent.name.clone(),
                    href: desk_path(&record.id, &agent.id),
                })
                .collect(),
            starter_action: format!("/projects/{}/agents/starter", record.id.as_hex()),
            starter_href: format!("/agents/new?project={}", record.id.as_hex()),
            grant_action: format!("/projects/{}/agents/grant", record.id.as_hex()),
            grant_candidates: catalogue
                .iter()
                .map(|agent| GrantCandidate {
                    id: agent.id.as_hex(),
                    name: agent.name.clone(),
                    revision: agent.revision.to_string(),
                })
                .collect(),
            grant_alias: grant_alias.to_owned(),
            grant_access: grant_access.to_owned(),
            error,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum ProjectFormMode {
    Initial,
    Selected,
    Manual,
    Edit,
}

impl ProjectFormMode {
    pub(super) fn is_initial(self) -> bool {
        matches!(self, Self::Initial)
    }

    pub(super) fn is_selected(self) -> bool {
        matches!(self, Self::Selected)
    }

    pub(super) fn is_manual(self) -> bool {
        matches!(self, Self::Manual)
    }
}

#[derive(Template)]
#[template(path = "projects/templates/form.html")]
pub(super) struct ProjectFormView {
    pub(super) title: &'static str,
    pub(super) lead: &'static str,
    pub(super) mode: ProjectFormMode,
    pub(super) action: String,
    pub(super) submit: &'static str,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) host_path: String,
    pub(super) revision: String,
    pub(super) error: &'static str,
}

#[derive(Template)]
#[template(path = "projects/templates/form.html", block = "project_form")]
pub(super) struct ProjectFormContents<'a> {
    pub(super) mode: ProjectFormMode,
    pub(super) action: &'a str,
    pub(super) submit: &'static str,
    pub(super) name: &'a str,
    pub(super) path: &'a str,
    pub(super) host_path: &'a str,
    pub(super) revision: &'a str,
    pub(super) error: &'static str,
}

impl ProjectFormView {
    pub(super) fn initial(error: &'static str) -> Self {
        Self::create_state(
            ProjectFormMode::Initial,
            String::new(),
            String::new(),
            error,
        )
    }

    pub(super) fn selected(name: &str, path: &str, error: &'static str) -> Self {
        Self::create_state(
            ProjectFormMode::Selected,
            name.to_owned(),
            path.to_owned(),
            error,
        )
    }

    pub(super) fn manual(name: &str, path: &str, error: &'static str) -> Self {
        Self::create_state(
            ProjectFormMode::Manual,
            name.to_owned(),
            path.to_owned(),
            error,
        )
    }

    pub(super) fn edit(record: &ProjectRecord, name: &str, error: &'static str) -> Self {
        Self {
            title: "Rename project",
            lead: "Change the project name. The project folder stays fixed.",
            mode: ProjectFormMode::Edit,
            action: format!("/projects/{}/configuration", record.id.as_hex()),
            submit: "Save name",
            name: name.to_owned(),
            path: String::new(),
            host_path: record.host_path.to_string_lossy().into_owned(),
            revision: record.revision.to_string(),
            error,
        }
    }

    fn create_state(
        mode: ProjectFormMode,
        name: String,
        path: String,
        error: &'static str,
    ) -> Self {
        Self {
            title: "New project",
            lead: "Add an existing Git project from this machine. Power Plant does not create or clone repositories.",
            mode,
            action: "/projects".to_owned(),
            submit: "Add project",
            name,
            path,
            host_path: String::new(),
            revision: String::new(),
            error,
        }
    }

    pub(super) fn contents(&self) -> ProjectFormContents<'_> {
        ProjectFormContents {
            mode: self.mode,
            action: &self.action,
            submit: self.submit,
            name: &self.name,
            path: &self.path,
            host_path: &self.host_path,
            revision: &self.revision,
            error: self.error,
        }
    }
}
