use askama::Template;

use crate::projects::ProjectRecord;

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

#[derive(Template)]
#[template(path = "projects/templates/detail.html")]
pub(super) struct DetailView {
    pub(super) document_title: String,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) available: bool,
}

impl DetailView {
    pub(super) fn from_record(record: &ProjectRecord) -> Self {
        Self {
            document_title: format!("{} | Power Plant", record.name),
            id: record.id.as_hex(),
            name: record.name.clone(),
            path: record.host_path.to_string_lossy().into_owned(),
            available: record.host_path_is_available(),
        }
    }
}

#[derive(Template)]
#[template(path = "projects/templates/form.html")]
pub(super) struct ProjectFormView {
    pub(super) title: &'static str,
    pub(super) lead: &'static str,
    pub(super) action: String,
    pub(super) submit: &'static str,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) host_path: String,
    pub(super) show_path: bool,
    pub(super) revision: String,
    pub(super) error: &'static str,
}

#[derive(Template)]
#[template(path = "projects/templates/form.html", block = "project_form")]
pub(super) struct ProjectFormContents<'a> {
    pub(super) action: &'a str,
    pub(super) submit: &'static str,
    pub(super) name: &'a str,
    pub(super) path: &'a str,
    pub(super) host_path: &'a str,
    pub(super) show_path: bool,
    pub(super) revision: &'a str,
    pub(super) error: &'static str,
}

impl ProjectFormView {
    pub(super) fn create(name: &str, path: &str, error: &'static str) -> Self {
        Self {
            title: "New project",
            lead: "Register a local Git worktree. The path stays fixed after creation.",
            action: "/projects".to_owned(),
            submit: "Create project",
            name: name.to_owned(),
            path: path.to_owned(),
            host_path: String::new(),
            show_path: true,
            revision: String::new(),
            error,
        }
    }

    pub(super) fn edit(record: &ProjectRecord, name: &str, error: &'static str) -> Self {
        Self {
            title: "Rename project",
            lead: "Change the project name. The host path stays fixed.",
            action: format!("/projects/{}/configuration", record.id.as_hex()),
            submit: "Save name",
            name: name.to_owned(),
            path: String::new(),
            host_path: record.host_path.to_string_lossy().into_owned(),
            show_path: false,
            revision: record.revision.to_string(),
            error,
        }
    }

    pub(super) fn contents(&self) -> ProjectFormContents<'_> {
        ProjectFormContents {
            action: &self.action,
            submit: self.submit,
            name: &self.name,
            path: &self.path,
            host_path: &self.host_path,
            show_path: self.show_path,
            revision: &self.revision,
            error: self.error,
        }
    }
}
