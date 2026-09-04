use crate::agents::{AccessMode, ToolId};
use crate::workflows::definition::{
    AgentAuthority, AgentStep, ArtefactKind, ArtefactSource, CandidateAuthority,
    GuestDirectoryAccess, HumanGateStep, InputKey, MAXIMUM_DIRECTORIES, MAXIMUM_INPUTS,
    MAXIMUM_OUTPUTS, MAXIMUM_ROLES, MAXIMUM_STEPS, OutputKey, OutputKind, RequiredInput,
    RequiredOutput, ReviewPolicy, RoleDefinition, RoleKey, StepAction, StepDefinition,
    StepEnvironment, StepKey, SystemCommandId, SystemCommandStep, WorkflowDefinition,
    candidate_revision_output, initial_candidate_input,
};
use crate::workflows::{CatalogueError, WorkflowRecord};

pub(super) const MAXIMUM_FORM_BYTES: usize = 768 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FormError {
    Intent,
    Index,
    UnknownField,
    DuplicateField,
    MissingField,
    Sparse,
    Excessive,
    Revision,
    ReviewPolicy,
    ReviewTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FormIntent {
    Save,
    AddRole,
    RemoveRole(usize),
    MoveRoleUp(usize),
    MoveRoleDown(usize),
    AddStep,
    RemoveStep(usize),
    MoveStepUp(usize),
    MoveStepDown(usize),
    UpdateReviewPolicy(usize),
    AddDirectory { step: usize },
    RemoveDirectory { step: usize, directory: usize },
    AddOutput(usize),
    RemoveOutput { step: usize, output: usize },
    MoveOutputUp { step: usize, output: usize },
    MoveOutputDown { step: usize, output: usize },
    AddInput(usize),
    RemoveInput { step: usize, input: usize },
}

#[derive(Clone, Debug)]
pub(super) struct RoleDraft {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) expertise: String,
    pub(super) prompt: String,
}

#[derive(Clone, Debug)]
pub(super) struct DirectoryDraft {
    pub(super) alias: String,
    pub(super) access: String,
}

#[derive(Clone, Debug)]
pub(super) struct OutputDraft {
    pub(super) key: String,
    pub(super) kind: String,
}

#[derive(Clone, Debug)]
pub(super) struct InputDraft {
    pub(super) key: String,
    pub(super) kind: String,
    pub(super) source: String,
}

#[derive(Clone, Debug)]
pub(super) struct ReviewPolicyDraft {
    pub(super) report_output: String,
    pub(super) revision_target: String,
    pub(super) attempt_limit: String,
}

#[derive(Clone, Debug)]
pub(super) struct StepDraft {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) action: String,
    pub(super) environment: String,
    pub(super) role: String,
    pub(super) candidate_access: String,
    pub(super) command: String,
    pub(super) tools: Vec<ToolId>,
    pub(super) directories: Vec<DirectoryDraft>,
    pub(super) inputs: Vec<InputDraft>,
    pub(super) outputs: Vec<OutputDraft>,
    pub(super) review_policy: Option<ReviewPolicyDraft>,
}

#[derive(Clone, Debug)]
pub(super) struct WorkflowFormState {
    pub(super) name: String,
    pub(super) default_environment: String,
    pub(super) revision: Option<u64>,
    pub(super) roles: Vec<RoleDraft>,
    pub(super) steps: Vec<StepDraft>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RoleErrors {
    pub(super) key: &'static str,
    pub(super) name: &'static str,
    pub(super) expertise: &'static str,
    pub(super) prompt: &'static str,
}

#[derive(Clone, Debug, Default)]
pub(super) struct DirectoryErrors {
    pub(super) alias: &'static str,
}

#[derive(Clone, Debug, Default)]
pub(super) struct OutputErrors {
    pub(super) key: &'static str,
    pub(super) kind: &'static str,
}

#[derive(Clone, Debug, Default)]
pub(super) struct InputErrors {
    pub(super) key: &'static str,
    pub(super) kind: &'static str,
    pub(super) source: &'static str,
}

#[derive(Clone, Debug, Default)]
pub(super) struct StepErrors {
    pub(super) key: &'static str,
    pub(super) name: &'static str,
    pub(super) action: &'static str,
    pub(super) environment: &'static str,
    pub(super) role: &'static str,
    pub(super) candidate_access: &'static str,
    pub(super) command: &'static str,
    pub(super) review_policy: &'static str,
    pub(super) report_output: &'static str,
    pub(super) revision_target: &'static str,
    pub(super) attempt_limit: &'static str,
    pub(super) directories: Vec<DirectoryErrors>,
    pub(super) inputs: Vec<InputErrors>,
    pub(super) outputs: Vec<OutputErrors>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct FormErrors {
    pub(super) summary: &'static str,
    pub(super) name: &'static str,
    pub(super) default_environment: &'static str,
    pub(super) roles: Vec<RoleErrors>,
    pub(super) steps: Vec<StepErrors>,
}

impl FormError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Intent => "That form action is not valid.",
            Self::ReviewPolicy => "That review policy is not valid.",
            Self::Index => "That form row is not valid.",
            Self::UnknownField => "That form includes an unknown field.",
            Self::DuplicateField => "That form includes a duplicate field.",
            Self::MissingField => "That form omits a required field.",
            Self::Sparse => "That form row is not valid.",
            Self::Excessive => "That form has too many rows.",
            Self::Revision => "Reload the workflow and try again.",
            Self::ReviewTarget => "That action would invalidate a review revision target.",
        }
    }
}

impl FormErrors {
    pub(super) fn summary(message: &'static str) -> Self {
        Self {
            summary: message,
            ..Self::default()
        }
    }

    pub(super) fn for_state(state: &WorkflowFormState) -> Self {
        Self::sized(state.roles.len(), &state.steps)
    }

    fn sized(roles: usize, steps: &[StepDraft]) -> Self {
        Self {
            summary: "",
            name: "",
            default_environment: "",
            roles: vec![RoleErrors::default(); roles],
            steps: steps
                .iter()
                .map(|step| StepErrors {
                    directories: vec![DirectoryErrors::default(); step.directories.len()],
                    inputs: vec![InputErrors::default(); step.inputs.len()],
                    outputs: vec![OutputErrors::default(); step.outputs.len()],
                    ..StepErrors::default()
                })
                .collect(),
        }
    }

    fn has_field_error(&self) -> bool {
        !self.name.is_empty()
            || !self.default_environment.is_empty()
            || self.roles.iter().any(|role| {
                !role.key.is_empty()
                    || !role.name.is_empty()
                    || !role.expertise.is_empty()
                    || !role.prompt.is_empty()
            })
            || self.steps.iter().any(|step| {
                !step.key.is_empty()
                    || !step.name.is_empty()
                    || !step.action.is_empty()
                    || !step.environment.is_empty()
                    || !step.role.is_empty()
                    || !step.candidate_access.is_empty()
                    || !step.command.is_empty()
                    || !step.review_policy.is_empty()
                    || !step.report_output.is_empty()
                    || !step.revision_target.is_empty()
                    || !step.attempt_limit.is_empty()
                    || step
                        .directories
                        .iter()
                        .any(|directory| !directory.alias.is_empty())
                    || step.inputs.iter().any(|input| {
                        !input.key.is_empty() || !input.kind.is_empty() || !input.source.is_empty()
                    })
                    || step
                        .outputs
                        .iter()
                        .any(|output| !output.key.is_empty() || !output.kind.is_empty())
            })
    }
}

impl WorkflowFormState {
    pub(super) fn blank() -> Self {
        Self {
            name: String::new(),
            default_environment: String::new(),
            revision: None,
            roles: vec![RoleDraft {
                key: "role-1".to_owned(),
                name: String::new(),
                expertise: String::new(),
                prompt: String::new(),
            }],
            steps: vec![blank_agent_step("step-1", "role-1")],
        }
    }

    pub(super) fn from_record(record: &WorkflowRecord) -> Self {
        let roles = record
            .definition
            .roles()
            .iter()
            .map(|role| RoleDraft {
                key: role.key.as_str().to_owned(),
                name: role.name.clone(),
                expertise: role.expertise.clone(),
                prompt: role.prompt_defaults.clone(),
            })
            .collect();
        let steps = record
            .definition
            .steps()
            .iter()
            .map(step_from_definition)
            .collect();
        Self {
            name: record.definition.name().to_owned(),
            default_environment: record.definition.default_environment().as_hex(),
            revision: Some(record.revision),
            roles,
            steps,
        }
    }

    pub(super) fn maintain_candidate_outputs_from(&mut self, previous: &Self) {
        let mut changed = false;
        for step in &mut self.steps {
            let Some(previous_step) = previous
                .steps
                .iter()
                .find(|previous_step| previous_step.key == step.key)
            else {
                continue;
            };
            if step.action != "agent"
                || previous_step.candidate_access == CandidateAuthority::Edit.as_str()
                || step.candidate_access != CandidateAuthority::Edit.as_str()
                || step.outputs.iter().any(|output| {
                    OutputKind::parse(&output.kind) == Some(OutputKind::CandidateRevision)
                })
            {
                continue;
            }
            let key = next_key(
                "candidate",
                &step
                    .outputs
                    .iter()
                    .map(|output| output.key.as_str())
                    .collect::<Vec<_>>(),
            );
            step.outputs.push(OutputDraft {
                key,
                kind: OutputKind::CandidateRevision.as_str().to_owned(),
            });
            changed = true;
        }
        if !changed {
            return;
        }
        let mut latest = "run-initial-candidate".to_owned();
        for step in &mut self.steps {
            for input in &mut step.inputs {
                if ArtefactKind::parse(&input.kind) == Some(ArtefactKind::CandidateRevision)
                    && input.source != "run-current-candidate"
                {
                    input.source = latest.clone();
                }
            }
            if let Some(output) = step.outputs.iter().find(|output| {
                OutputKind::parse(&output.kind) == Some(OutputKind::CandidateRevision)
            }) {
                latest = format!("step-output:{}:{}", step.key, output.key);
            }
        }
    }

    pub(super) fn parse(pairs: Vec<(String, String)>) -> Result<(Self, FormIntent), FormError> {
        let mut seen = Vec::new();
        let mut name = String::new();
        let mut default_environment = String::new();
        let mut revision = None;
        let mut intent = None;
        let mut role_fields: Vec<(usize, RolePart, String)> = Vec::new();
        let mut step_fields: Vec<(usize, StepPart, String)> = Vec::new();
        for (key, value) in pairs {
            if seen.iter().any(|item: &String| item == &key) {
                return Err(FormError::DuplicateField);
            }
            seen.push(key.clone());
            match parse_field(&key)? {
                Field::Name => name = value,
                Field::DefaultEnvironment => default_environment = value,
                Field::Revision => {
                    if !value.trim().is_empty() {
                        revision = Some(parse_revision(&value)?);
                    }
                }
                Field::Intent => intent = Some(parse_intent(&value)?),
                Field::Role { index, part } => role_fields.push((index, part, value)),
                Field::Step { index, part } => step_fields.push((index, part, value)),
            }
        }
        let intent = intent.ok_or(FormError::Intent)?;
        let roles = collect_roles(role_fields)?;
        let mut steps = collect_steps(step_fields)?;
        if roles.len() > MAXIMUM_ROLES || steps.len() > MAXIMUM_STEPS {
            return Err(FormError::Excessive);
        }
        ensure_action_defaults(&mut steps);
        Ok((
            Self {
                name,
                default_environment,
                revision,
                roles,
                steps,
            },
            intent,
        ))
    }

    pub(super) fn apply(&mut self, intent: FormIntent) -> Result<(), FormError> {
        match intent {
            FormIntent::Save => Ok(()),
            FormIntent::AddRole => {
                if self.roles.len() >= MAXIMUM_ROLES {
                    return Err(FormError::Excessive);
                }
                let key = next_key(
                    "role",
                    &self
                        .roles
                        .iter()
                        .map(|role| role.key.as_str())
                        .collect::<Vec<_>>(),
                );
                self.roles.push(RoleDraft {
                    key,
                    name: String::new(),
                    expertise: String::new(),
                    prompt: String::new(),
                });
                Ok(())
            }
            FormIntent::RemoveRole(index) => {
                if self.roles.len() <= 1 || index >= self.roles.len() {
                    return Err(FormError::Index);
                }
                self.roles.remove(index);
                Ok(())
            }
            FormIntent::MoveRoleUp(index) => move_item(&mut self.roles, index, true),
            FormIntent::MoveRoleDown(index) => move_item(&mut self.roles, index, false),
            FormIntent::AddStep => {
                if self.steps.len() >= MAXIMUM_STEPS {
                    return Err(FormError::Excessive);
                }
                let key = next_key(
                    "step",
                    &self
                        .steps
                        .iter()
                        .map(|step| step.key.as_str())
                        .collect::<Vec<_>>(),
                );
                let role = self
                    .roles
                    .first()
                    .map(|role| role.key.clone())
                    .unwrap_or_else(|| "role-1".to_owned());
                let mut added = blank_agent_step(&key, &role);
                if let Some(input) = added.inputs.first_mut() {
                    input.source = latest_candidate_source(&self.steps);
                }
                self.steps.push(added);
                Ok(())
            }
            FormIntent::RemoveStep(index) => {
                if self.steps.len() <= 1 || index >= self.steps.len() {
                    return Err(FormError::Index);
                }
                if !can_remove_step(&self.steps, index) {
                    return Err(FormError::ReviewTarget);
                }
                self.steps.remove(index);
                Ok(())
            }
            FormIntent::MoveStepUp(index) => move_step(&mut self.steps, index, true),
            FormIntent::MoveStepDown(index) => move_step(&mut self.steps, index, false),
            FormIntent::UpdateReviewPolicy(index) => {
                if index >= self.steps.len() {
                    return Err(FormError::Index);
                }
                Ok(())
            }
            FormIntent::AddDirectory { step } => {
                let row = self.steps.get_mut(step).ok_or(FormError::Index)?;
                if row.directories.len() >= MAXIMUM_DIRECTORIES {
                    return Err(FormError::Excessive);
                }
                row.directories.push(DirectoryDraft {
                    alias: String::new(),
                    access: AccessMode::ReadOnly.as_str().to_owned(),
                });
                Ok(())
            }
            FormIntent::RemoveDirectory { step, directory } => {
                let row = self.steps.get_mut(step).ok_or(FormError::Index)?;
                if directory >= row.directories.len() {
                    return Err(FormError::Index);
                }
                row.directories.remove(directory);
                Ok(())
            }
            FormIntent::AddInput(step) => {
                if step >= self.steps.len() {
                    return Err(FormError::Index);
                }
                if self.steps[step].inputs.len() >= MAXIMUM_INPUTS {
                    return Err(FormError::Excessive);
                }
                let source = latest_candidate_source(&self.steps[..step]);
                self.steps[step].inputs.push(InputDraft {
                    key: String::new(),
                    kind: ArtefactKind::CandidateRevision.as_str().to_owned(),
                    source,
                });
                Ok(())
            }
            FormIntent::RemoveInput { step, input } => {
                let row = self.steps.get_mut(step).ok_or(FormError::Index)?;
                if input >= row.inputs.len() {
                    return Err(FormError::Index);
                }
                row.inputs.remove(input);
                Ok(())
            }
            FormIntent::AddOutput(step) => {
                let row = self.steps.get_mut(step).ok_or(FormError::Index)?;
                if row.outputs.len() >= MAXIMUM_OUTPUTS {
                    return Err(FormError::Excessive);
                }
                row.outputs.push(OutputDraft {
                    key: String::new(),
                    kind: OutputKind::AssistantReply.as_str().to_owned(),
                });
                Ok(())
            }
            FormIntent::RemoveOutput { step, output } => {
                let row = self.steps.get_mut(step).ok_or(FormError::Index)?;
                if row.outputs.len() <= 1 || output >= row.outputs.len() {
                    return Err(FormError::Index);
                }
                row.outputs.remove(output);
                Ok(())
            }
            FormIntent::MoveOutputUp { step, output } => {
                let row = self.steps.get_mut(step).ok_or(FormError::Index)?;
                move_item(&mut row.outputs, output, true)
            }
            FormIntent::MoveOutputDown { step, output } => {
                let row = self.steps.get_mut(step).ok_or(FormError::Index)?;
                move_item(&mut row.outputs, output, false)
            }
        }
    }

    pub(super) fn to_definition(&self) -> Result<WorkflowDefinition, FormErrors> {
        let mut errors = FormErrors::sized(self.roles.len(), &self.steps);
        let mut roles = Vec::new();
        for (index, role) in self.roles.iter().enumerate() {
            match RoleKey::parse(&role.key) {
                Ok(key) => match RoleDefinition::new(
                    key,
                    role.name.clone(),
                    role.expertise.clone(),
                    role.prompt.clone(),
                ) {
                    Ok(role) => roles.push(role),
                    Err(error) => match error {
                        crate::workflows::definition::DefinitionError::Name => {
                            errors.roles[index].name = error.message();
                        }
                        crate::workflows::definition::DefinitionError::Expertise => {
                            errors.roles[index].expertise = error.message();
                        }
                        crate::workflows::definition::DefinitionError::PromptDefaults => {
                            errors.roles[index].prompt = error.message();
                        }
                        _ => errors.roles[index].key = error.message(),
                    },
                },
                Err(error) => errors.roles[index].key = error.message(),
            }
        }
        let mut steps = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            if let Some(built) = build_step(step, &mut errors.steps[index]) {
                steps.push(built);
            }
        }
        if !errors.summary.is_empty() || errors.has_field_error() {
            if errors.summary.is_empty() {
                errors.summary = "Fix the highlighted fields.";
            }
            return Err(errors);
        }
        if steps.is_empty() {
            errors.summary = crate::workflows::definition::DefinitionError::StepCount.message();
            return Err(errors);
        }
        for (index, (step, draft)) in steps.iter_mut().zip(&self.steps).enumerate() {
            step.review = if let Some(policy) = &draft.review_policy {
                let report_output = match OutputKey::parse(&policy.report_output) {
                    Ok(key) => key,
                    Err(error) => {
                        errors.steps[index].report_output = error.message();
                        errors.summary = "Fix the highlighted fields.";
                        return Err(errors);
                    }
                };
                let revision_target = match StepKey::parse(&policy.revision_target) {
                    Ok(key) => key,
                    Err(error) => {
                        errors.steps[index].revision_target = error.message();
                        errors.summary = "Fix the highlighted fields.";
                        return Err(errors);
                    }
                };
                let attempt_limit = match policy.attempt_limit.parse::<u8>() {
                    Ok(limit)
                        if (crate::workflows::definition::MINIMUM_REVIEW_ATTEMPTS
                            ..=crate::workflows::definition::MAXIMUM_REVIEW_ATTEMPTS)
                            .contains(&limit) =>
                    {
                        limit
                    }
                    _ => {
                        errors.steps[index].attempt_limit =
                            crate::workflows::definition::DefinitionError::AttemptLimit.message();
                        errors.summary = "Fix the highlighted fields.";
                        return Err(errors);
                    }
                };
                Some(ReviewPolicy {
                    report_output,
                    revision_target,
                    attempt_limit,
                })
            } else {
                None
            };
        }
        let default_environment =
            match crate::environments::EnvironmentId::parse(self.default_environment.trim()) {
                Some(id) => id,
                None => {
                    errors.default_environment =
                        crate::workflows::definition::DefinitionError::Environment.message();
                    errors.summary = "Fix the highlighted fields.";
                    return Err(errors);
                }
            };
        match WorkflowDefinition::from_parts(self.name.clone(), default_environment, roles, steps) {
            Ok(definition) => Ok(definition),
            Err(error) => {
                relate_definition_error(self, error, &mut errors);
                if errors.summary.is_empty() {
                    errors.summary = error.message();
                }
                Err(errors)
            }
        }
    }
}

impl From<CatalogueError> for FormErrors {
    fn from(error: CatalogueError) -> Self {
        let mut errors = FormErrors::summary(error.message());
        if error == CatalogueError::DuplicateName {
            errors.name = error.message();
        }
        errors
    }
}

#[derive(Clone, Copy)]
enum Field {
    Name,
    DefaultEnvironment,
    Revision,
    Intent,
    Role { index: usize, part: RolePart },
    Step { index: usize, part: StepPart },
}

#[derive(Clone, Copy)]
enum RolePart {
    Key,
    Name,
    Expertise,
    Prompt,
}

#[derive(Clone, Copy)]
enum StepPart {
    Key,
    Name,
    Action,
    Environment,
    Role,
    CandidateAccess,
    Command,
    ReviewPolicy,
    ReportOutput,
    RevisionTarget,
    AttemptLimit,
    Tool(ToolId),
    Dir { index: usize, part: DirPart },
    Input { index: usize, part: InputPart },
    Output { index: usize, part: OutputPart },
}

#[derive(Clone, Copy)]
enum DirPart {
    Alias,
    Access,
}

#[derive(Clone, Copy)]
enum OutputPart {
    Key,
    Kind,
}

#[derive(Clone, Copy)]
enum InputPart {
    Key,
    Kind,
    Source,
}

fn parse_field(name: &str) -> Result<Field, FormError> {
    match name {
        "name" => Ok(Field::Name),
        "default-environment" => Ok(Field::DefaultEnvironment),
        "revision" => Ok(Field::Revision),
        "intent" => Ok(Field::Intent),
        _ => parse_row_field(name),
    }
}

fn parse_row_field(name: &str) -> Result<Field, FormError> {
    let mut parts = name.split('_');
    let prefix = parts.next().ok_or(FormError::UnknownField)?;
    match prefix {
        "role" => {
            let index = parse_index(parts.next().ok_or(FormError::UnknownField)?)?;
            let part = match parts.next() {
                Some("key") => RolePart::Key,
                Some("name") => RolePart::Name,
                Some("expertise") => RolePart::Expertise,
                Some("prompt") => RolePart::Prompt,
                _ => return Err(FormError::UnknownField),
            };
            if parts.next().is_some() {
                return Err(FormError::UnknownField);
            }
            Ok(Field::Role { index, part })
        }
        "step" => {
            let index = parse_index(parts.next().ok_or(FormError::UnknownField)?)?;
            let part = match parts.next() {
                Some("key") => StepPart::Key,
                Some("name") => StepPart::Name,
                Some("action") => StepPart::Action,
                Some("environment") => StepPart::Environment,
                Some("role") => StepPart::Role,
                Some("candidate-access") => StepPart::CandidateAccess,
                Some("command") => StepPart::Command,
                Some("review-policy") => StepPart::ReviewPolicy,
                Some("report-output") => StepPart::ReportOutput,
                Some("revision-target") => StepPart::RevisionTarget,
                Some("attempt-limit") => StepPart::AttemptLimit,
                Some("tool") => {
                    let tool = parts.next().ok_or(FormError::UnknownField)?;
                    let tool = ToolId::parse(tool).ok_or(FormError::UnknownField)?;
                    if parts.next().is_some() {
                        return Err(FormError::UnknownField);
                    }
                    return Ok(Field::Step {
                        index,
                        part: StepPart::Tool(tool),
                    });
                }
                Some("dir") => {
                    let dir = parse_index(parts.next().ok_or(FormError::UnknownField)?)?;
                    let dir_part = match parts.next() {
                        Some("alias") => DirPart::Alias,
                        Some("access") => DirPart::Access,
                        _ => return Err(FormError::UnknownField),
                    };
                    if parts.next().is_some() {
                        return Err(FormError::UnknownField);
                    }
                    StepPart::Dir {
                        index: dir,
                        part: dir_part,
                    }
                }
                Some("input") => {
                    let input = parse_index(parts.next().ok_or(FormError::UnknownField)?)?;
                    let input_part = match parts.next() {
                        Some("key") => InputPart::Key,
                        Some("kind") => InputPart::Kind,
                        Some("source") => InputPart::Source,
                        _ => return Err(FormError::UnknownField),
                    };
                    if parts.next().is_some() {
                        return Err(FormError::UnknownField);
                    }
                    StepPart::Input {
                        index: input,
                        part: input_part,
                    }
                }
                Some("output") => {
                    let output = parse_index(parts.next().ok_or(FormError::UnknownField)?)?;
                    let output_part = match parts.next() {
                        Some("key") => OutputPart::Key,
                        Some("kind") => OutputPart::Kind,
                        _ => return Err(FormError::UnknownField),
                    };
                    if parts.next().is_some() {
                        return Err(FormError::UnknownField);
                    }
                    StepPart::Output {
                        index: output,
                        part: output_part,
                    }
                }
                _ => return Err(FormError::UnknownField),
            };
            if !matches!(
                part,
                StepPart::Dir { .. } | StepPart::Input { .. } | StepPart::Output { .. }
            ) && parts.next().is_some()
            {
                return Err(FormError::UnknownField);
            }
            Ok(Field::Step { index, part })
        }
        _ => Err(FormError::UnknownField),
    }
}

fn parse_index(raw: &str) -> Result<usize, FormError> {
    if raw.is_empty() || (raw.len() > 1 && raw.starts_with('0')) {
        return Err(FormError::Index);
    }
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(FormError::Index);
    }
    raw.parse().map_err(|_| FormError::Index)
}

fn parse_revision(raw: &str) -> Result<u64, FormError> {
    let raw = raw.trim();
    if raw.is_empty() || (raw.len() > 1 && raw.starts_with('0')) {
        return Err(FormError::Revision);
    }
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(FormError::Revision);
    }
    raw.parse().map_err(|_| FormError::Revision)
}

fn parse_review_policy(raw: &str) -> Result<bool, FormError> {
    match raw {
        "none" => Ok(false),
        "review-verdict" => Ok(true),
        _ => Err(FormError::ReviewPolicy),
    }
}

fn parse_intent(raw: &str) -> Result<FormIntent, FormError> {
    match raw {
        "save" => Ok(FormIntent::Save),
        "add-role" => Ok(FormIntent::AddRole),
        "add-step" => Ok(FormIntent::AddStep),
        other => parse_indexed_intent(other),
    }
}

fn parse_indexed_intent(raw: &str) -> Result<FormIntent, FormError> {
    let mut parts = raw.split(':');
    let action = parts.next().ok_or(FormError::Intent)?;
    let first = parse_index(parts.next().ok_or(FormError::Intent)?)?;
    match action {
        "remove-role" if parts.next().is_none() => Ok(FormIntent::RemoveRole(first)),
        "move-role-up" if parts.next().is_none() => Ok(FormIntent::MoveRoleUp(first)),
        "move-role-down" if parts.next().is_none() => Ok(FormIntent::MoveRoleDown(first)),
        "remove-step" if parts.next().is_none() => Ok(FormIntent::RemoveStep(first)),
        "move-step-up" if parts.next().is_none() => Ok(FormIntent::MoveStepUp(first)),
        "move-step-down" if parts.next().is_none() => Ok(FormIntent::MoveStepDown(first)),
        "update-review-policy" if parts.next().is_none() => {
            Ok(FormIntent::UpdateReviewPolicy(first))
        }
        "add-directory" if parts.next().is_none() => Ok(FormIntent::AddDirectory { step: first }),
        "remove-directory" => {
            let directory = parse_index(parts.next().ok_or(FormError::Intent)?)?;
            if parts.next().is_some() {
                return Err(FormError::Intent);
            }
            Ok(FormIntent::RemoveDirectory {
                step: first,
                directory,
            })
        }
        "add-input" if parts.next().is_none() => Ok(FormIntent::AddInput(first)),
        "remove-input" => {
            let input = parse_index(parts.next().ok_or(FormError::Intent)?)?;
            if parts.next().is_some() {
                return Err(FormError::Intent);
            }
            Ok(FormIntent::RemoveInput { step: first, input })
        }
        "add-output" if parts.next().is_none() => Ok(FormIntent::AddOutput(first)),
        "remove-output" => {
            let output = parse_index(parts.next().ok_or(FormError::Intent)?)?;
            if parts.next().is_some() {
                return Err(FormError::Intent);
            }
            Ok(FormIntent::RemoveOutput {
                step: first,
                output,
            })
        }
        "move-output-up" => {
            let output = parse_index(parts.next().ok_or(FormError::Intent)?)?;
            if parts.next().is_some() {
                return Err(FormError::Intent);
            }
            Ok(FormIntent::MoveOutputUp {
                step: first,
                output,
            })
        }
        "move-output-down" => {
            let output = parse_index(parts.next().ok_or(FormError::Intent)?)?;
            if parts.next().is_some() {
                return Err(FormError::Intent);
            }
            Ok(FormIntent::MoveOutputDown {
                step: first,
                output,
            })
        }
        _ => Err(FormError::Intent),
    }
}

fn collect_roles(fields: Vec<(usize, RolePart, String)>) -> Result<Vec<RoleDraft>, FormError> {
    let count = dense_count(fields.iter().map(|(index, _, _)| *index))?;
    if count > MAXIMUM_ROLES {
        return Err(FormError::Excessive);
    }
    let mut roles = vec![
        RoleDraft {
            key: String::new(),
            name: String::new(),
            expertise: String::new(),
            prompt: String::new(),
        };
        count
    ];
    for (index, part, value) in fields {
        let role = &mut roles[index];
        match part {
            RolePart::Key => role.key = value,
            RolePart::Name => role.name = value,
            RolePart::Expertise => role.expertise = value,
            RolePart::Prompt => role.prompt = value,
        }
    }
    Ok(roles)
}

fn collect_steps(fields: Vec<(usize, StepPart, String)>) -> Result<Vec<StepDraft>, FormError> {
    let count = dense_count(fields.iter().map(|(index, _, _)| *index))?;
    if count > MAXIMUM_STEPS {
        return Err(FormError::Excessive);
    }
    let mut steps = vec![empty_step(); count];
    let mut dir_seen = vec![Vec::<usize>::new(); count];
    let mut input_seen = vec![Vec::<usize>::new(); count];
    let mut output_seen = vec![Vec::<usize>::new(); count];
    let mut review_policy = vec![None; count];
    let mut review_drafts = vec![empty_review_policy(); count];
    let mut review_detail_seen = vec![[false; 3]; count];
    for (index, part, value) in fields {
        let step = &mut steps[index];
        match part {
            StepPart::Key => step.key = value,
            StepPart::Name => step.name = value,
            StepPart::Action => step.action = value,
            StepPart::Environment => step.environment = value,
            StepPart::Role => step.role = value,
            StepPart::CandidateAccess => step.candidate_access = value,
            StepPart::Command => step.command = value,
            StepPart::ReviewPolicy => {
                review_policy[index] = Some(parse_review_policy(&value)?);
            }
            StepPart::ReportOutput => {
                review_drafts[index].report_output = value;
                review_detail_seen[index][0] = true;
            }
            StepPart::RevisionTarget => {
                review_drafts[index].revision_target = value;
                review_detail_seen[index][1] = true;
            }
            StepPart::AttemptLimit => {
                review_drafts[index].attempt_limit = value;
                review_detail_seen[index][2] = true;
            }
            StepPart::Tool(tool) => {
                if is_checked(&value) && !step.tools.contains(&tool) {
                    step.tools.push(tool);
                }
            }
            StepPart::Dir {
                index: dir,
                part: dir_part,
            } => {
                ensure_row(&mut step.directories, dir, || DirectoryDraft {
                    alias: String::new(),
                    access: AccessMode::ReadOnly.as_str().to_owned(),
                })?;
                dir_seen[index].push(dir);
                match dir_part {
                    DirPart::Alias => step.directories[dir].alias = value,
                    DirPart::Access => step.directories[dir].access = value,
                }
            }
            StepPart::Input {
                index: input,
                part: input_part,
            } => {
                ensure_row(&mut step.inputs, input, || InputDraft {
                    key: String::new(),
                    kind: ArtefactKind::CandidateRevision.as_str().to_owned(),
                    source: String::new(),
                })?;
                input_seen[index].push(input);
                match input_part {
                    InputPart::Key => step.inputs[input].key = value,
                    InputPart::Kind => step.inputs[input].kind = value,
                    InputPart::Source => step.inputs[input].source = value,
                }
            }
            StepPart::Output {
                index: output,
                part: output_part,
            } => {
                ensure_row(&mut step.outputs, output, || OutputDraft {
                    key: String::new(),
                    kind: OutputKind::AssistantReply.as_str().to_owned(),
                })?;
                output_seen[index].push(output);
                match output_part {
                    OutputPart::Key => step.outputs[output].key = value,
                    OutputPart::Kind => step.outputs[output].kind = value,
                }
            }
        }
    }
    for index in 0..steps.len() {
        let Some(has_review_policy) = review_policy[index] else {
            return Err(FormError::MissingField);
        };
        let details_present = review_detail_seen[index].iter().any(|seen| *seen);
        let details_complete = review_detail_seen[index].iter().all(|seen| *seen);
        if details_present && !details_complete {
            return Err(FormError::MissingField);
        }
        if has_review_policy {
            steps[index].review_policy = Some(review_drafts[index].clone());
        }
        let step = &steps[index];
        if !dir_seen[index].is_empty() {
            dense_count(dir_seen[index].iter().copied())?;
            if step.directories.len() > MAXIMUM_DIRECTORIES {
                return Err(FormError::Excessive);
            }
        }
        if !input_seen[index].is_empty() {
            dense_count(input_seen[index].iter().copied())?;
            if step.inputs.len() > MAXIMUM_INPUTS {
                return Err(FormError::Excessive);
            }
        }
        if !output_seen[index].is_empty() {
            dense_count(output_seen[index].iter().copied())?;
            if step.outputs.len() > MAXIMUM_OUTPUTS {
                return Err(FormError::Excessive);
            }
        }
    }
    Ok(steps)
}

fn dense_count(indices: impl Iterator<Item = usize>) -> Result<usize, FormError> {
    let mut max = None;
    let mut seen = Vec::new();
    for index in indices {
        if !seen.contains(&index) {
            seen.push(index);
        }
        max = Some(max.map_or(index, |current: usize| current.max(index)));
    }
    let Some(max) = max else {
        return Ok(0);
    };
    let count = max.checked_add(1).ok_or(FormError::Excessive)?;
    if seen.len() != count {
        return Err(FormError::Sparse);
    }
    Ok(count)
}

fn ensure_row<T>(rows: &mut Vec<T>, index: usize, make: impl Fn() -> T) -> Result<(), FormError> {
    if index >= MAXIMUM_DIRECTORIES.max(MAXIMUM_OUTPUTS).max(MAXIMUM_INPUTS) {
        return Err(FormError::Excessive);
    }
    while rows.len() <= index {
        rows.push(make());
    }
    Ok(())
}

fn is_checked(value: &str) -> bool {
    matches!(value, "on" | "true" | "1")
}

fn ensure_action_defaults(steps: &mut [StepDraft]) {
    for index in 0..steps.len() {
        if steps[index].action == "human-gate" {
            steps[index].environment.clear();
            steps[index].role.clear();
            steps[index].candidate_access.clear();
            steps[index].command.clear();
            steps[index].tools.clear();
            steps[index].directories.clear();
            let candidate_source = latest_candidate_source(&steps[..index]);
            if !steps[index]
                .inputs
                .iter()
                .any(|input| input.kind == ArtefactKind::CandidateRevision.as_str())
            {
                steps[index].inputs.insert(
                    0,
                    InputDraft {
                        key: "candidate".to_owned(),
                        kind: ArtefactKind::CandidateRevision.as_str().to_owned(),
                        source: candidate_source,
                    },
                );
            }
            steps[index].outputs = vec![OutputDraft {
                key: "decision".to_owned(),
                kind: OutputKind::HumanDecision.as_str().to_owned(),
            }];
            continue;
        }
        if steps[index].action != "system-command" {
            continue;
        }
        let Some(command) = SystemCommandId::parse(&steps[index].command) else {
            continue;
        };
        steps[index].role.clear();
        steps[index].candidate_access.clear();
        steps[index].tools.clear();
        steps[index].directories.clear();
        let contract = command.contract();
        let candidate_source = latest_candidate_source(&steps[..index]);
        let existing_inputs = std::mem::take(&mut steps[index].inputs);
        let mut required_inputs = contract.required_inputs.to_vec();
        if command == SystemCommandId::CommitCandidate {
            let review_count = existing_inputs
                .iter()
                .filter(|input| {
                    ArtefactKind::parse(&input.kind) == Some(ArtefactKind::ReviewReport)
                })
                .count();
            let has_decision = existing_inputs
                .iter()
                .any(|input| ArtefactKind::parse(&input.kind) == Some(ArtefactKind::HumanDecision));
            if has_decision && review_count == 0 {
                required_inputs.retain(|kind| *kind != ArtefactKind::ReviewReport);
            } else {
                required_inputs.extend(std::iter::repeat_n(
                    ArtefactKind::ReviewReport,
                    review_count.saturating_sub(1),
                ));
            }
            if has_decision {
                required_inputs.push(ArtefactKind::HumanDecision);
            }
        }
        steps[index].inputs = required_inputs
            .iter()
            .enumerate()
            .map(|(position, kind)| {
                existing_inputs
                    .iter()
                    .filter(|input| ArtefactKind::parse(&input.kind) == Some(*kind))
                    .nth(
                        required_inputs[..position]
                            .iter()
                            .filter(|prior| **prior == *kind)
                            .count(),
                    )
                    .cloned()
                    .unwrap_or_else(|| InputDraft {
                        key: match kind {
                            ArtefactKind::CandidateRevision => "candidate",
                            ArtefactKind::ReviewReport => "review",
                            ArtefactKind::Plan => "plan",
                            ArtefactKind::TestReport => "test",
                            ArtefactKind::HumanDecision => "decision",
                        }
                        .to_owned(),
                        kind: kind.as_str().to_owned(),
                        source: if position == 0 && *kind == ArtefactKind::CandidateRevision {
                            candidate_source.clone()
                        } else {
                            String::new()
                        },
                    })
            })
            .collect();
        let existing_outputs = std::mem::take(&mut steps[index].outputs);
        steps[index].outputs = contract
            .required_outputs
            .iter()
            .map(|kind| {
                existing_outputs
                    .iter()
                    .find(|output| OutputKind::parse(&output.kind) == Some(*kind))
                    .cloned()
                    .unwrap_or_else(|| OutputDraft {
                        key: "committed-candidate".to_owned(),
                        kind: kind.as_str().to_owned(),
                    })
            })
            .collect();
    }
}

fn empty_review_policy() -> ReviewPolicyDraft {
    ReviewPolicyDraft {
        report_output: String::new(),
        revision_target: String::new(),
        attempt_limit: "3".to_owned(),
    }
}

fn empty_step() -> StepDraft {
    StepDraft {
        key: String::new(),
        name: String::new(),
        action: "agent".to_owned(),
        environment: String::new(),
        role: String::new(),
        candidate_access: CandidateAuthority::Edit.as_str().to_owned(),
        command: SystemCommandId::RepositoryStatus.as_str().to_owned(),
        tools: Vec::new(),
        directories: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        review_policy: None,
    }
}

fn blank_agent_step(key: &str, role: &str) -> StepDraft {
    StepDraft {
        key: key.to_owned(),
        name: String::new(),
        action: "agent".to_owned(),
        environment: String::new(),
        role: role.to_owned(),
        candidate_access: CandidateAuthority::Edit.as_str().to_owned(),
        command: SystemCommandId::RepositoryStatus.as_str().to_owned(),
        tools: ToolId::ALL.to_vec(),
        directories: Vec::new(),
        inputs: vec![input_from_required(&initial_candidate_input())],
        review_policy: None,
        outputs: vec![
            OutputDraft {
                key: "assistant-reply".to_owned(),
                kind: OutputKind::AssistantReply.as_str().to_owned(),
            },
            output_from_required(&candidate_revision_output()),
        ],
    }
}

fn latest_candidate_source(earlier: &[StepDraft]) -> String {
    for step in earlier.iter().rev() {
        if let Some(output) = step
            .outputs
            .iter()
            .rev()
            .find(|output| output.kind == OutputKind::CandidateRevision.as_str())
        {
            return format!("step-output:{}:{}", step.key, output.key);
        }
    }
    "run-initial-candidate".to_owned()
}

fn input_from_required(input: &RequiredInput) -> InputDraft {
    InputDraft {
        key: input.key.as_str().to_owned(),
        kind: input.kind.as_str().to_owned(),
        source: source_token(&input.source),
    }
}

fn output_from_required(output: &RequiredOutput) -> OutputDraft {
    OutputDraft {
        key: output.key.as_str().to_owned(),
        kind: output.kind.as_str().to_owned(),
    }
}

fn source_token(source: &ArtefactSource) -> String {
    match source {
        ArtefactSource::RunInitialCandidate => "run-initial-candidate".to_owned(),
        ArtefactSource::RunCurrentCandidate => "run-current-candidate".to_owned(),
        ArtefactSource::StepOutput { step, output } => {
            format!("step-output:{}:{}", step.as_str(), output.as_str())
        }
    }
}

fn parse_source(
    raw: &str,
) -> Result<ArtefactSource, crate::workflows::definition::DefinitionError> {
    if raw == "run-initial-candidate" {
        return Ok(ArtefactSource::RunInitialCandidate);
    }
    if raw == "run-current-candidate" {
        return Ok(ArtefactSource::RunCurrentCandidate);
    }
    let Some(rest) = raw.strip_prefix("step-output:") else {
        return Err(crate::workflows::definition::DefinitionError::Format);
    };
    let Some((step, output)) = rest.split_once(':') else {
        return Err(crate::workflows::definition::DefinitionError::Format);
    };
    Ok(ArtefactSource::StepOutput {
        step: StepKey::parse(step)?,
        output: OutputKey::parse(output)?,
    })
}

fn step_from_definition(step: &StepDefinition) -> StepDraft {
    let review_policy = step.review.as_ref().map(|policy| ReviewPolicyDraft {
        report_output: policy.report_output.as_str().to_owned(),
        revision_target: policy.revision_target.as_str().to_owned(),
        attempt_limit: policy.attempt_limit.to_string(),
    });
    match &step.action {
        StepAction::Agent(action) => {
            let directories: Vec<DirectoryDraft> = action
                .authority
                .directories
                .iter()
                .map(|directory| DirectoryDraft {
                    alias: directory.alias.clone(),
                    access: directory.access.as_str().to_owned(),
                })
                .collect();
            StepDraft {
                key: step.key.as_str().to_owned(),
                name: step.name.clone(),
                action: "agent".to_owned(),
                environment: step_environment_token(action.environment),
                role: action.role.as_str().to_owned(),
                candidate_access: action.candidate_authority.as_str().to_owned(),
                command: SystemCommandId::RepositoryStatus.as_str().to_owned(),
                tools: action.authority.tools.clone(),
                directories,
                inputs: step.inputs.iter().map(input_from_required).collect(),
                outputs: action
                    .required_outputs
                    .iter()
                    .map(output_from_required)
                    .collect(),
                review_policy: review_policy.clone(),
            }
        }
        StepAction::SystemCommand(action) => StepDraft {
            key: step.key.as_str().to_owned(),
            name: step.name.clone(),
            action: "system-command".to_owned(),
            environment: step_environment_token(action.environment),
            role: String::new(),
            candidate_access: String::new(),
            command: action.command.as_str().to_owned(),
            tools: Vec::new(),
            directories: Vec::new(),
            inputs: step.inputs.iter().map(input_from_required).collect(),
            outputs: action
                .required_outputs
                .iter()
                .map(output_from_required)
                .collect(),
            review_policy: review_policy.clone(),
        },
        StepAction::HumanGate(action) => StepDraft {
            key: step.key.as_str().to_owned(),
            name: step.name.clone(),
            action: "human-gate".to_owned(),
            environment: String::new(),
            role: String::new(),
            candidate_access: String::new(),
            command: String::new(),
            tools: Vec::new(),
            directories: Vec::new(),
            inputs: step.inputs.iter().map(input_from_required).collect(),
            outputs: vec![output_from_required(&action.required_output)],
            review_policy,
        },
    }
}

fn build_step(step: &StepDraft, errors: &mut StepErrors) -> Option<StepDefinition> {
    let key = match StepKey::parse(&step.key) {
        Ok(key) => key,
        Err(error) => {
            errors.key = error.message();
            return None;
        }
    };
    let display_name = step.name.trim().to_owned();
    if display_name.is_empty()
        || display_name.len() > 80
        || display_name.chars().any(char::is_control)
    {
        errors.name = crate::workflows::definition::DefinitionError::Name.message();
        return None;
    }
    let action = match step.action.as_str() {
        "agent" => build_agent_action(step, errors)?,
        "system-command" => build_command_action(step, errors)?,
        "human-gate" => build_gate_action(step, errors)?,
        _ => {
            errors.action = "Choose an action type.";
            return None;
        }
    };
    let mut inputs = Vec::new();
    for (index, input) in step.inputs.iter().enumerate() {
        let key = match InputKey::parse(&input.key) {
            Ok(key) => key,
            Err(error) => {
                errors.inputs[index].key = error.message();
                return None;
            }
        };
        let Some(kind) = ArtefactKind::parse(&input.kind) else {
            errors.inputs[index].kind =
                crate::workflows::definition::DefinitionError::Format.message();
            return None;
        };
        let source = match parse_source(&input.source) {
            Ok(source) => source,
            Err(error) => {
                errors.inputs[index].source = error.message();
                return None;
            }
        };
        inputs.push(RequiredInput { key, kind, source });
    }
    Some(StepDefinition {
        key,
        name: display_name,
        inputs,
        action,
        review: None,
    })
}

fn build_agent_action(step: &StepDraft, errors: &mut StepErrors) -> Option<StepAction> {
    let role = match RoleKey::parse(&step.role) {
        Ok(role) => role,
        Err(error) => {
            errors.role = error.message();
            return None;
        }
    };
    let candidate_authority = match CandidateAuthority::parse(&step.candidate_access) {
        Some(authority) => authority,
        None => {
            errors.candidate_access = "Choose candidate access.";
            return None;
        }
    };
    let mut directories = Vec::new();
    for directory in &step.directories {
        if directory.alias.trim().is_empty() {
            continue;
        }
        directories.push(GuestDirectoryAccess {
            alias: directory.alias.clone(),
            access: AccessMode::ReadOnly,
        });
    }
    let authority = match AgentAuthority::new(step.tools.clone(), directories) {
        Ok(authority) => authority,
        Err(error) => {
            errors.action = error.message();
            return None;
        }
    };
    let mut outputs = Vec::new();
    for (index, output) in step.outputs.iter().enumerate() {
        let key = match OutputKey::parse(&output.key) {
            Ok(key) => key,
            Err(error) => {
                errors.outputs[index].key = error.message();
                return None;
            }
        };
        let Some(kind) = OutputKind::parse(&output.kind) else {
            errors.outputs[index].kind =
                crate::workflows::definition::DefinitionError::Format.message();
            return None;
        };
        outputs.push(RequiredOutput { key, kind });
    }
    let candidate_outputs = outputs
        .iter()
        .filter(|output| output.kind == OutputKind::CandidateRevision)
        .count();
    match candidate_authority {
        CandidateAuthority::ReadOnly if candidate_outputs != 0 => {
            errors.candidate_access = "A read-only step cannot produce a candidate revision.";
            return None;
        }
        CandidateAuthority::Edit if candidate_outputs != 1 => {
            errors.candidate_access = "An edit step needs one candidate revision output.";
            return None;
        }
        _ => {}
    }
    Some(StepAction::Agent(AgentStep {
        environment: parse_step_environment(&step.environment, errors)?,
        role,
        candidate_authority,
        authority,
        required_outputs: outputs,
    }))
}

fn build_gate_action(step: &StepDraft, errors: &mut StepErrors) -> Option<StepAction> {
    if step.outputs.len() != 1 {
        errors.action = crate::workflows::definition::DefinitionError::HumanGate.message();
        return None;
    }
    let output = &step.outputs[0];
    let key = match OutputKey::parse(&output.key) {
        Ok(key) => key,
        Err(error) => {
            errors.outputs[0].key = error.message();
            return None;
        }
    };
    if OutputKind::parse(&output.kind) != Some(OutputKind::HumanDecision) {
        errors.outputs[0].kind = crate::workflows::definition::DefinitionError::HumanGate.message();
        return None;
    }
    Some(StepAction::HumanGate(HumanGateStep {
        required_output: RequiredOutput {
            key,
            kind: OutputKind::HumanDecision,
        },
    }))
}

fn build_command_action(step: &StepDraft, errors: &mut StepErrors) -> Option<StepAction> {
    let Some(command) = SystemCommandId::parse(&step.command) else {
        errors.command = crate::workflows::definition::DefinitionError::Command.message();
        return None;
    };
    let mut outputs = Vec::new();
    for (index, output) in step.outputs.iter().enumerate() {
        let key = match OutputKey::parse(&output.key) {
            Ok(key) => key,
            Err(error) => {
                errors.outputs[index].key = error.message();
                return None;
            }
        };
        let Some(kind) = OutputKind::parse(&output.kind) else {
            errors.outputs[index].kind =
                crate::workflows::definition::DefinitionError::Format.message();
            return None;
        };
        outputs.push(RequiredOutput { key, kind });
    }
    let contract = command.contract();
    let output_kinds: Vec<_> = outputs.iter().map(|output| output.kind).collect();
    if !crate::workflows::commands::kinds_match(&output_kinds, contract.required_outputs) {
        if let Some(index) = (0..outputs.len()).find(|index| {
            contract
                .required_outputs
                .get(*index)
                .is_none_or(|kind| outputs[*index].kind != *kind)
        }) {
            errors.outputs[index].kind =
                crate::workflows::definition::DefinitionError::UnsupportedOutput.message();
        } else {
            errors.command =
                crate::workflows::definition::DefinitionError::UnsupportedOutput.message();
        }
        return None;
    }
    Some(StepAction::SystemCommand(SystemCommandStep {
        environment: parse_step_environment(&step.environment, errors)?,
        command,
        required_outputs: outputs,
    }))
}

fn step_environment_token(environment: StepEnvironment) -> String {
    match environment {
        StepEnvironment::WorkflowDefault => String::new(),
        StepEnvironment::Override { environment_id } => environment_id.as_hex(),
    }
}

fn parse_step_environment(raw: &str, errors: &mut StepErrors) -> Option<StepEnvironment> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(StepEnvironment::WorkflowDefault);
    }
    match crate::environments::EnvironmentId::parse(raw) {
        Some(environment_id) => Some(StepEnvironment::Override { environment_id }),
        None => {
            errors.environment =
                crate::workflows::definition::DefinitionError::Environment.message();
            None
        }
    }
}

fn relate_definition_error(
    state: &WorkflowFormState,
    error: crate::workflows::definition::DefinitionError,
    errors: &mut FormErrors,
) {
    use crate::workflows::definition::DefinitionError;
    errors.summary = error.message();
    match error {
        DefinitionError::Name => errors.name = error.message(),
        DefinitionError::DuplicateRole => {
            for role in &mut errors.roles {
                if role.key.is_empty() {
                    role.key = error.message();
                }
            }
        }
        DefinitionError::DuplicateStep => {
            for step in &mut errors.steps {
                if step.key.is_empty() {
                    step.key = error.message();
                }
            }
        }
        DefinitionError::UnknownRole => {
            for (index, step) in state.steps.iter().enumerate() {
                if step.action == "agent" && !state.roles.iter().any(|role| role.key == step.role) {
                    errors.steps[index].role = error.message();
                }
            }
        }
        DefinitionError::UnsupportedOutput => {
            for (index, step) in state.steps.iter().enumerate() {
                if step.action != "system-command" {
                    continue;
                }
                let Some(command) = SystemCommandId::parse(&step.command) else {
                    continue;
                };
                let contract = command.contract();
                let input_kinds: Vec<_> = step
                    .inputs
                    .iter()
                    .filter_map(|input| ArtefactKind::parse(&input.kind))
                    .collect();
                let output_kinds: Vec<_> = step
                    .outputs
                    .iter()
                    .filter_map(|output| OutputKind::parse(&output.kind))
                    .collect();
                if !contract.accepts(&input_kinds, &output_kinds) {
                    mark_command_input_errors(
                        &mut errors.steps[index],
                        step,
                        contract.required_inputs,
                    );
                }
                if !crate::workflows::commands::kinds_match(
                    &output_kinds,
                    contract.required_outputs,
                ) {
                    mark_command_output_errors(
                        &mut errors.steps[index],
                        step,
                        contract.required_outputs,
                    );
                }
            }
        }
        _ => {}
    }
}

fn mark_command_input_errors(errors: &mut StepErrors, step: &StepDraft, required: &[ArtefactKind]) {
    let missing_candidate = !required.contains(&ArtefactKind::CandidateRevision)
        || !step
            .inputs
            .iter()
            .any(|input| input.kind == ArtefactKind::CandidateRevision.as_str());
    if missing_candidate && let Some(error) = errors.inputs.first_mut() {
        error.kind = "Add one candidate input.";
    }
    if required.contains(&ArtefactKind::ReviewReport)
        && !step
            .inputs
            .iter()
            .any(|input| input.kind == ArtefactKind::ReviewReport.as_str())
    {
        if let Some(error) = errors.inputs.get_mut(1) {
            error.kind = "Add one review report input.";
        } else if let Some(error) = errors.inputs.first_mut() {
            error.kind = "Add one review report input.";
        }
    }
    for (index, input) in step.inputs.iter().enumerate() {
        if ArtefactKind::parse(&input.kind)
            .is_some_and(|kind| !required.contains(&kind) && kind != ArtefactKind::HumanDecision)
        {
            errors.inputs[index].kind =
                crate::workflows::definition::DefinitionError::InputKind.message();
        }
    }
}

fn mark_command_output_errors(errors: &mut StepErrors, step: &StepDraft, required: &[OutputKind]) {
    for (index, output) in step.outputs.iter().enumerate() {
        if OutputKind::parse(&output.kind).is_some_and(|kind| !required.contains(&kind)) {
            errors.outputs[index].kind =
                crate::workflows::definition::DefinitionError::UnsupportedOutput.message();
        }
    }
    if step.outputs.is_empty() && !required.is_empty() {
        errors.command = crate::workflows::definition::DefinitionError::UnsupportedOutput.message();
    }
}

fn next_key(prefix: &str, existing: &[&str]) -> String {
    for ordinal in 1..=existing.len() + 1 {
        let key = format!("{prefix}-{ordinal}");
        if !existing.contains(&key.as_str()) {
            return key;
        }
    }
    format!("{prefix}-x")
}

pub(super) fn can_move_step(steps: &[StepDraft], index: usize, up: bool) -> bool {
    let Ok(target) = move_target(steps.len(), index, up) else {
        return false;
    };
    let mut moved = steps.to_vec();
    moved.swap(index, target);
    review_targets_are_earlier(&moved)
}

pub(super) fn can_remove_step(steps: &[StepDraft], index: usize) -> bool {
    if steps.len() <= 1 || index >= steps.len() {
        return false;
    }
    let mut remaining = steps.to_vec();
    remaining.remove(index);
    review_targets_are_earlier(&remaining)
}

fn move_step(steps: &mut [StepDraft], index: usize, up: bool) -> Result<(), FormError> {
    let target = move_target(steps.len(), index, up)?;
    if !can_move_step(steps, index, up) {
        return Err(FormError::ReviewTarget);
    }
    steps.swap(index, target);
    Ok(())
}

fn review_targets_are_earlier(steps: &[StepDraft]) -> bool {
    steps.iter().enumerate().all(|(index, step)| {
        step.review_policy.as_ref().is_none_or(|policy| {
            steps[..index]
                .iter()
                .any(|candidate| candidate.key == policy.revision_target)
        })
    })
}

fn move_item<T>(items: &mut [T], index: usize, up: bool) -> Result<(), FormError> {
    let target = move_target(items.len(), index, up)?;
    items.swap(index, target);
    Ok(())
}

fn move_target(len: usize, index: usize, up: bool) -> Result<usize, FormError> {
    if index >= len {
        return Err(FormError::Index);
    }
    if up {
        return index.checked_sub(1).ok_or(FormError::Index);
    }
    let target = index.checked_add(1).ok_or(FormError::Index)?;
    (target < len).then_some(target).ok_or(FormError::Index)
}

pub(super) fn parse_delete(pairs: &[(String, String)]) -> Result<(u64, bool), FormError> {
    let mut revision = None;
    let mut confirm = false;
    let mut seen = Vec::new();
    for (key, value) in pairs {
        if seen.contains(&key) {
            return Err(FormError::DuplicateField);
        }
        seen.push(key);
        match key.as_str() {
            "revision" => revision = Some(parse_revision(value)?),
            "confirm" => confirm = is_checked(value),
            _ => return Err(FormError::UnknownField),
        }
    }
    Ok((revision.ok_or(FormError::Revision)?, confirm))
}

#[cfg(test)]
mod tests;
