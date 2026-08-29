use askama::Template;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::agents::ToolId;
use crate::workflows::WorkflowRecord;
use crate::workflows::definition::{MAXIMUM_INPUTS, MAXIMUM_OUTPUTS, MAXIMUM_ROLES, MAXIMUM_STEPS};

use super::forms::{FormErrors, RoleDraft, StepDraft, WorkflowFormState};

pub(super) struct EnvironmentOption {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) context: String,
    pub(super) selected: bool,
}

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

pub(super) struct InputRow {
    pub(super) index: usize,
    pub(super) key: String,
    pub(super) key_error: &'static str,
    pub(super) kind: String,
    pub(super) kind_error: &'static str,
    pub(super) source_error: &'static str,
    pub(super) sources: Vec<SourceOption>,
    pub(super) can_remove: bool,
}

pub(super) struct SourceOption {
    pub(super) value: String,
    pub(super) label: String,
    pub(super) selected: bool,
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
    pub(super) environment: String,
    pub(super) environment_error: &'static str,
    pub(super) environment_hint: String,
    pub(super) action_error: &'static str,
    pub(super) role: String,
    pub(super) role_error: &'static str,
    pub(super) command: String,
    pub(super) command_error: &'static str,
    pub(super) tools: Vec<ToolChoice>,
    pub(super) directories: Vec<DirectoryRow>,
    pub(super) inputs: Vec<InputRow>,
    pub(super) can_add_input: bool,
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
    pub(super) default_environment: String,
    pub(super) default_environment_error: &'static str,
    pub(super) environment_options: Vec<EnvironmentOption>,
    pub(super) no_ready_environment: bool,
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
    pub(super) default_environment: &'a str,
    pub(super) default_environment_error: &'static str,
    pub(super) environment_options: &'a [EnvironmentOption],
    pub(super) no_ready_environment: bool,
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
            name: state.name.clone(),
            name_error: errors.name,
            default_environment: state.default_environment.clone(),
            default_environment_error: errors.default_environment,
            environment_options: Vec::new(),
            no_ready_environment: false,
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
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    step_row(
                        index,
                        step_count,
                        step,
                        &state.steps[..index],
                        errors.steps.get(index),
                    )
                })
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

    pub(super) fn with_environments(
        mut self,
        options: Vec<EnvironmentOption>,
        no_ready: bool,
    ) -> Self {
        let default_name = options
            .iter()
            .find(|option| option.id == self.default_environment)
            .map(|option| option.name.clone())
            .unwrap_or_default();
        for step in &mut self.steps {
            if step.environment.is_empty() {
                step.environment_hint = if default_name.is_empty() {
                    String::new()
                } else {
                    format!("Uses {default_name}")
                };
            } else if let Some(option) = options.iter().find(|option| option.id == step.environment)
            {
                step.environment_hint = option.context.clone();
            }
        }
        self.environment_options = options;
        self.no_ready_environment = no_ready;
        self
    }

    pub(super) fn contents(&self) -> WorkflowFormContents<'_> {
        WorkflowFormContents {
            action: &self.action,
            submit: self.submit,
            name: &self.name,
            name_error: self.name_error,
            default_environment: &self.default_environment,
            default_environment_error: self.default_environment_error,
            environment_options: &self.environment_options,
            no_ready_environment: self.no_ready_environment,
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
    step: &StepDraft,
    earlier: &[StepDraft],
    errors: Option<&super::forms::StepErrors>,
) -> StepRow {
    let errors = errors.cloned().unwrap_or_default();
    let output_count = step.outputs.len();
    let input_count = step.inputs.len();
    let is_agent = step.action != "system-command";
    StepRow {
        index,
        key: step.key.clone(),
        key_error: errors.key,
        name: step.name.clone(),
        name_error: errors.name,
        is_agent,
        environment: step.environment.clone(),
        environment_error: errors.environment,
        environment_hint: String::new(),
        action_error: errors.action,
        role: step.role.clone(),
        role_error: errors.role,
        command: step.command.clone(),
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
            .iter()
            .enumerate()
            .map(|(dir_index, directory)| DirectoryRow {
                index: dir_index,
                alias: directory.alias.clone(),
                alias_error: errors
                    .directories
                    .get(dir_index)
                    .map(|item| item.alias)
                    .unwrap_or(""),
                read_write: directory.access != "read-only",
            })
            .collect(),
        inputs: step
            .inputs
            .iter()
            .enumerate()
            .map(|(input_index, input)| InputRow {
                index: input_index,
                key: input.key.clone(),
                key_error: errors
                    .inputs
                    .get(input_index)
                    .map(|item| item.key)
                    .unwrap_or(""),
                kind: input.kind.clone(),
                kind_error: errors
                    .inputs
                    .get(input_index)
                    .map(|item| item.kind)
                    .unwrap_or(""),
                source_error: errors
                    .inputs
                    .get(input_index)
                    .map(|item| item.source)
                    .unwrap_or(""),
                sources: source_options(earlier, &input.kind, &input.source),
                can_remove: input_count > 0,
            })
            .collect(),
        can_add_input: input_count < MAXIMUM_INPUTS,
        outputs: step
            .outputs
            .iter()
            .enumerate()
            .map(|(output_index, output)| OutputRow {
                index: output_index,
                key: output.key.clone(),
                key_error: errors
                    .outputs
                    .get(output_index)
                    .map(|item| item.key)
                    .unwrap_or(""),
                kind: output.kind.clone(),
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

fn source_options(earlier: &[StepDraft], kind: &str, current: &str) -> Vec<SourceOption> {
    let mut options = Vec::new();
    if kind == "candidate-revision" {
        options.push(SourceOption {
            value: "run-initial-candidate".to_owned(),
            label: "Task source candidate".to_owned(),
            selected: current == "run-initial-candidate",
        });
    }
    for step in earlier {
        for output in &step.outputs {
            if output.kind != kind || output.kind == "assistant-reply" {
                continue;
            }
            let value = format!("step-output:{}:{}", step.key, output.key);
            let selected = current == value;
            options.push(SourceOption {
                value,
                label: format!("{} · {}", step.key, output.key),
                selected,
            });
        }
    }
    if !current.is_empty() && !options.iter().any(|option| option.selected) {
        options.push(SourceOption {
            value: current.to_owned(),
            label: current.to_owned(),
            selected: true,
        });
    }
    options
}

fn format_time(ms: u64) -> String {
    let seconds = i64::try_from(ms / 1000).unwrap_or(0);
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| time.format(&Rfc3339).ok())
        .unwrap_or_else(|| "unknown".to_owned())
}
