use askama::Template;

use crate::conversations::ConversationRecord;
use crate::projects::{ProjectId, ProjectRecord};

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

pub(super) struct ConversationLink {
    pub(super) id: String,
    pub(super) title: String,
}

#[derive(Template)]
#[template(path = "projects/templates/detail.html")]
pub(super) struct DetailView {
    pub(super) document_title: String,
    pub(super) project_id: String,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) available: bool,
    pub(super) conversations: Vec<ConversationLink>,
    pub(super) error: &'static str,
}

impl DetailView {
    pub(super) fn with_conversations(
        record: &ProjectRecord,
        conversations: &[ConversationRecord],
    ) -> Self {
        Self::base(record, project_conversations(record.id, conversations))
    }

    pub(super) fn with_error(
        record: &ProjectRecord,
        conversations: &[ConversationRecord],
        error: &'static str,
    ) -> Self {
        let mut view = Self::with_conversations(record, conversations);
        view.error = error;
        view
    }

    fn base(record: &ProjectRecord, conversations: Vec<ConversationLink>) -> Self {
        Self {
            document_title: format!("{} | Power Plant", record.name),
            project_id: record.id.as_hex(),
            name: record.name.clone(),
            path: record.host_path.to_string_lossy().into_owned(),
            available: record.host_path_is_available(),
            conversations,
            error: "",
        }
    }
}

fn project_conversations(
    project_id: ProjectId,
    records: &[ConversationRecord],
) -> Vec<ConversationLink> {
    let mut conversations: Vec<_> = records
        .iter()
        .filter(|record| record.projects.contains(&project_id))
        .map(|record| ConversationLink {
            id: record.id.as_hex(),
            title: record.title.clone(),
        })
        .collect();
    conversations.sort_by(|left, right| left.title.cmp(&right.title));
    conversations
}

#[derive(Clone, Copy)]
pub(super) enum ProjectFormMode {
    Create,
    Edit,
}

impl ProjectFormMode {
    pub(super) fn is_create(self) -> bool {
        matches!(self, Self::Create)
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
    pub(super) fn create(name: &str, path: &str, error: &'static str) -> Self {
        Self {
            title: "New project",
            lead: "Add an existing Git project from this machine.",
            mode: ProjectFormMode::Create,
            action: "/projects".to_owned(),
            submit: "Add project",
            name: name.to_owned(),
            path: path.to_owned(),
            host_path: String::new(),
            revision: String::new(),
            error,
        }
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
