use askama::Template;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::agents::ToolId;
use crate::workflows::WorkflowRecord;
use crate::workflows::definition::{MAXIMUM_OUTPUTS, MAXIMUM_ROLES, MAXIMUM_STEPS};

use super::forms::{FormErrors, RoleDraft, StepDraft, WorkflowFormState};

pub(super) const INDEX_TITLE: &str = "Workflows | Power Plant";
pub(super) const NEW_TITLE: &str = "New workflow | Power Plant";
pub(super) const CONFIG_TITLE: &str = "Configure workflow | Power Plant";

pub(super) struct CatalogueItem {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) roles: usize,
    pub(super) steps: usize,
    pub(super) version: String,
    pub(super) updated: String,
}

#[derive(Template)]
#[template(path = "workflows/templates/index.html")]
pub(super) struct CatalogueView {
    pub(super) workflows: Vec<CatalogueItem>,
}

impl CatalogueView {
    pub(super) fn from_records(records: &[WorkflowRecord]) -> Self {
        Self {
            workflows: records
                .iter()
                .map(|record| CatalogueItem {
                    id: record.id.as_hex(),
                    name: record.definition.name().to_owned(),
                    roles: record.definition.roles().len(),
                    steps: record.definition.steps().len(),
                    version: record.definition_version.short_hex(),
                    updated: format_time(record.updated_at_ms),
                })
                .collect(),
        }
    }
}

pub(super) struct ToolChoice {
    pub(super) name: &'static str,
    pub(super) label: &'static str,
    pub(super) checked: bool,
}

pub(super) struct DirectoryRow {
    pub(super) index: usize,
    pub(super) alias: String,
    pub(super) alias_error: &'static str,
    pub(super) read_write: bool,
}

pub(super) struct OutputRow {
    pub(super) index: usize,
    pub(super) key: String,
    pub(super) key_error: &'static str,
    pub(super) kind: String,
    pub(super) kind_error: &'static str,
    pub(super) can_move_up: bool,
    pub(super) can_move_down: bool,
    pub(super) can_remove: bool,
}

pub(super) struct RoleRow {
    pub(super) index: usize,
    pub(super) key: String,
    pub(super) key_error: &'static str,
    pub(super) name: String,
    pub(super) name_error: &'static str,
    pub(super) expertise: String,
    pub(super) expertise_error: &'static str,
    pub(super) prompt: String,
    pub(super) prompt_error: &'static str,
    pub(super) can_move_up: bool,
    pub(super) can_move_down: bool,
    pub(super) can_remove: bool,
}

pub(super) struct StepRow {
    pub(super) index: usize,
    pub(super) key: String,
    pub(super) key_error: &'static str,
    pub(super) name: String,
    pub(super) name_error: &'static str,
    pub(super) is_agent: bool,
    pub(super) action_error: &'static str,
    pub(super) role: String,
    pub(super) role_error: &'static str,
    pub(super) command: String,
    pub(super) command_error: &'static str,
    pub(super) tools: Vec<ToolChoice>,
    pub(super) directories: Vec<DirectoryRow>,
    pub(super) outputs: Vec<OutputRow>,
    pub(super) can_add_output: bool,
    pub(super) transition: &'static str,
    pub(super) can_move_up: bool,
    pub(super) can_move_down: bool,
    pub(super) can_remove: bool,
}

#[derive(Template)]
#[template(path = "workflows/templates/form.html")]
pub(super) struct WorkflowFormView {
    pub(super) title: &'static str,
    pub(super) action: String,
    pub(super) submit: &'static str,
    pub(super) name: String,
    pub(super) name_error: &'static str,
    pub(super) revision: String,
    pub(super) version: String,
    pub(super) summary_error: &'static str,
    pub(super) roles: Vec<RoleRow>,
    pub(super) steps: Vec<StepRow>,
    pub(super) can_add_role: bool,
    pub(super) can_add_step: bool,
    pub(super) workflow_id: String,
    pub(super) show_delete: bool,
    pub(super) delete_error: &'static str,
}

#[derive(Template)]
#[template(path = "workflows/templates/form.html", block = "workflow_form")]
pub(super) struct WorkflowFormContents<'a> {
    pub(super) action: &'a str,
    pub(super) submit: &'static str,
    pub(super) name: &'a str,
    pub(super) name_error: &'static str,
    pub(super) revision: &'a str,
    pub(super) version: &'a str,
    pub(super) summary_error: &'static str,
    pub(super) roles: &'a [RoleRow],
    pub(super) steps: &'a [StepRow],
    pub(super) can_add_role: bool,
    pub(super) can_add_step: bool,
    pub(super) workflow_id: &'a str,
    pub(super) show_delete: bool,
    pub(super) delete_error: &'static str,
}

impl WorkflowFormView {
    pub(super) fn create(state: WorkflowFormState, errors: FormErrors) -> Self {
        Self::from_state(
            "New workflow",
            "/workflows",
            "Create workflow",
            state,
            errors,
            "",
            "",
            false,
            "",
        )
    }

    pub(super) fn edit(
        record: &WorkflowRecord,
        state: WorkflowFormState,
        errors: FormErrors,
        delete_error: &'static str,
    ) -> Self {
        Self::from_state(
            "Configure workflow",
            &format!("/workflows/{}/configuration", record.id.as_hex()),
            "Save",
            state,
            errors,
            &record.revision.to_string(),
            &record.definition_version.as_hex(),
            true,
            delete_error,
        )
    }

    pub(super) fn edit_state(
        record: &WorkflowRecord,
        errors: FormErrors,
        delete_error: &'static str,
    ) -> Self {
        Self::edit(
            record,
            WorkflowFormState::from_record(record),
            errors,
            delete_error,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_state(
        title: &'static str,
        action: &str,
        submit: &'static str,
        state: WorkflowFormState,
        errors: FormErrors,
        revision: &str,
        version: &str,
        show_delete: bool,
        delete_error: &'static str,
    ) -> Self {
        let role_count = state.roles.len();
        let step_count = state.steps.len();
        let revision = state
            .revision
            .map(|value| value.to_string())
            .unwrap_or_else(|| revision.to_owned());
        Self {
            title,
            action: action.to_owned(),
            submit,
            name: state.name,
            name_error: errors.name,
            revision,
            version: version.to_owned(),
            summary_error: errors.summary,
            roles: state
                .roles
                .into_iter()
                .enumerate()
                .map(|(index, role)| role_row(index, role_count, role, errors.roles.get(index)))
                .collect(),
            steps: state
                .steps
                .into_iter()
                .enumerate()
                .map(|(index, step)| step_row(index, step_count, step, errors.steps.get(index)))
                .collect(),
            can_add_role: role_count < MAXIMUM_ROLES,
            can_add_step: step_count < MAXIMUM_STEPS,
            workflow_id: if show_delete {
                action
                    .trim_start_matches("/workflows/")
                    .trim_end_matches("/configuration")
                    .to_owned()
            } else {
                String::new()
            },
            show_delete,
            delete_error,
        }
    }

    pub(super) fn contents(&self) -> WorkflowFormContents<'_> {
        WorkflowFormContents {
            action: &self.action,
            submit: self.submit,
            name: &self.name,
            name_error: self.name_error,
            revision: &self.revision,
            version: &self.version,
            summary_error: self.summary_error,
            roles: &self.roles,
            steps: &self.steps,
            can_add_role: self.can_add_role,
            can_add_step: self.can_add_step,
            workflow_id: &self.workflow_id,
            show_delete: self.show_delete,
            delete_error: self.delete_error,
        }
    }
}

fn role_row(
    index: usize,
    count: usize,
    role: RoleDraft,
    errors: Option<&super::forms::RoleErrors>,
) -> RoleRow {
    let errors = errors.cloned().unwrap_or_default();
    RoleRow {
        index,
        key: role.key,
        key_error: errors.key,
        name: role.name,
        name_error: errors.name,
        expertise: role.expertise,
        expertise_error: errors.expertise,
        prompt: role.prompt,
        prompt_error: errors.prompt,
        can_move_up: index > 0,
        can_move_down: index + 1 < count,
        can_remove: count > 1,
    }
}

fn step_row(
    index: usize,
    count: usize,
    step: StepDraft,
    errors: Option<&super::forms::StepErrors>,
) -> StepRow {
    let errors = errors.cloned().unwrap_or_default();
    let output_count = step.outputs.len();
    let is_agent = step.action != "system-command";
    StepRow {
        index,
        key: step.key,
        key_error: errors.key,
        name: step.name,
        name_error: errors.name,
        is_agent,
        action_error: errors.action,
        role: step.role,
        role_error: errors.role,
        command: step.command,
        command_error: errors.command,
        tools: ToolId::ALL
            .into_iter()
            .map(|tool| ToolChoice {
                name: tool.as_str(),
                label: tool.label(),
                checked: step.tools.contains(&tool),
            })
            .collect(),
        directories: step
            .directories
            .into_iter()
            .enumerate()
            .map(|(dir_index, directory)| DirectoryRow {
                index: dir_index,
                alias: directory.alias,
                alias_error: errors
                    .directories
                    .get(dir_index)
                    .map(|item| item.alias)
                    .unwrap_or(""),
                read_write: directory.access != "read-only",
            })
            .collect(),
        outputs: step
            .outputs
            .into_iter()
            .enumerate()
            .map(|(output_index, output)| OutputRow {
                index: output_index,
                key: output.key,
                key_error: errors
                    .outputs
                    .get(output_index)
                    .map(|item| item.key)
                    .unwrap_or(""),
                kind: output.kind,
                kind_error: errors
                    .outputs
                    .get(output_index)
                    .map(|item| item.kind)
                    .unwrap_or(""),
                can_move_up: output_index > 0,
                can_move_down: output_index + 1 < output_count,
                can_remove: output_count > 1,
            })
            .collect(),
        can_add_output: output_count < MAXIMUM_OUTPUTS,
        transition: if index + 1 < count {
            "Then"
        } else {
            "Complete run"
        },
        can_move_up: index > 0,
        can_move_down: index + 1 < count,
        can_remove: count > 1,
    }
}

fn format_time(ms: u64) -> String {
    let seconds = i64::try_from(ms / 1000).unwrap_or(0);
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| time.format(&Rfc3339).ok())
        .unwrap_or_else(|| "unknown".to_owned())
}
