use crate::agents::{AccessMode, ToolId};
use crate::workflows::definition::{
    AgentAuthority, AgentStep, GuestDirectoryAccess, MAXIMUM_DIRECTORIES, MAXIMUM_OUTPUTS,
    MAXIMUM_ROLES, MAXIMUM_STEPS, OutputKey, OutputKind, RequiredOutput, RoleDefinition, RoleKey,
    StepAction, StepDefinition, StepKey, SuccessTransition, SystemCommandId, SystemCommandStep,
    WorkflowDefinition,
};
use crate::workflows::{CatalogueError, WorkflowRecord};

pub(super) const MAXIMUM_FORM_BYTES: usize = 768 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FormError {
    Intent,
    Index,
    UnknownField,
    DuplicateField,
    Sparse,
    Excessive,
    Revision,
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
    AddOutput(usize),
    RemoveOutput { step: usize, output: usize },
    MoveOutputUp { step: usize, output: usize },
    MoveOutputDown { step: usize, output: usize },
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
pub(super) struct StepDraft {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) action: String,
    pub(super) role: String,
    pub(super) command: String,
    pub(super) tools: Vec<ToolId>,
    pub(super) directories: Vec<DirectoryDraft>,
    pub(super) outputs: Vec<OutputDraft>,
}

#[derive(Clone, Debug)]
pub(super) struct WorkflowFormState {
    pub(super) name: String,
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
pub(super) struct StepErrors {
    pub(super) key: &'static str,
    pub(super) name: &'static str,
    pub(super) action: &'static str,
    pub(super) role: &'static str,
    pub(super) command: &'static str,
    pub(super) directories: Vec<DirectoryErrors>,
    pub(super) outputs: Vec<OutputErrors>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct FormErrors {
    pub(super) summary: &'static str,
    pub(super) name: &'static str,
    pub(super) roles: Vec<RoleErrors>,
    pub(super) steps: Vec<StepErrors>,
}

impl FormError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Intent => "That form action is not valid.",
            Self::Index => "That form row is not valid.",
            Self::UnknownField => "That form includes an unknown field.",
            Self::DuplicateField => "That form includes a duplicate field.",
            Self::Sparse => "That form row is not valid.",
            Self::Excessive => "That form has too many rows.",
            Self::Revision => "Reload the workflow and try again.",
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

    fn sized(roles: usize, steps: &[StepDraft]) -> Self {
        Self {
            summary: "",
            name: "",
            roles: vec![RoleErrors::default(); roles],
            steps: steps
                .iter()
                .map(|step| StepErrors {
                    directories: vec![DirectoryErrors::default(); step.directories.len()],
                    outputs: vec![OutputErrors::default(); step.outputs.len()],
                    ..StepErrors::default()
                })
                .collect(),
        }
    }

    fn has_field_error(&self) -> bool {
        !self.name.is_empty()
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
                    || !step.role.is_empty()
                    || !step.command.is_empty()
                    || step
                        .directories
                        .iter()
                        .any(|directory| !directory.alias.is_empty())
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
        let mut steps = Vec::new();
        let mut current = record.definition.first_step().clone();
        while let Some(step) = record.definition.step(&current) {
            steps.push(step_from_definition(step));
            match &step.on_success {
                SuccessTransition::Next(next) => current = next.clone(),
                SuccessTransition::CompleteRun => break,
            }
        }
        Self {
            name: record.definition.name().to_owned(),
            revision: Some(record.revision),
            roles,
            steps,
        }
    }

    pub(super) fn parse(pairs: Vec<(String, String)>) -> Result<(Self, FormIntent), FormError> {
        let mut seen = Vec::new();
        let mut name = String::new();
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
        let steps = collect_steps(step_fields)?;
        if roles.len() > MAXIMUM_ROLES || steps.len() > MAXIMUM_STEPS {
            return Err(FormError::Excessive);
        }
        Ok((
            Self {
                name,
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
                self.steps.push(blank_agent_step(&key, &role));
                Ok(())
            }
            FormIntent::RemoveStep(index) => {
                if self.steps.len() <= 1 || index >= self.steps.len() {
                    return Err(FormError::Index);
                }
                self.steps.remove(index);
                Ok(())
            }
            FormIntent::MoveStepUp(index) => move_item(&mut self.steps, index, true),
            FormIntent::MoveStepDown(index) => move_item(&mut self.steps, index, false),
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
        let first = steps[0].key.clone();
        let last = steps.len() - 1;
        for index in 0..last {
            steps[index].on_success = SuccessTransition::Next(steps[index + 1].key.clone());
        }
        steps[last].on_success = SuccessTransition::CompleteRun;
        match WorkflowDefinition::from_parts(self.name.clone(), roles, first, steps) {
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
    Role,
    Command,
    Tool(ToolId),
    Dir { index: usize, part: DirPart },
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

fn parse_field(name: &str) -> Result<Field, FormError> {
    match name {
        "name" => Ok(Field::Name),
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
                Some("role") => StepPart::Role,
                Some("command") => StepPart::Command,
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
            if !matches!(part, StepPart::Dir { .. } | StepPart::Output { .. })
                && parts.next().is_some()
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
    let mut output_seen = vec![Vec::<usize>::new(); count];
    for (index, part, value) in fields {
        let step = &mut steps[index];
        match part {
            StepPart::Key => step.key = value,
            StepPart::Name => step.name = value,
            StepPart::Action => step.action = value,
            StepPart::Role => step.role = value,
            StepPart::Command => step.command = value,
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
                    access: AccessMode::ReadWrite.as_str().to_owned(),
                })?;
                dir_seen[index].push(dir);
                match dir_part {
                    DirPart::Alias => step.directories[dir].alias = value,
                    DirPart::Access => step.directories[dir].access = value,
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
    for (index, step) in steps.iter().enumerate() {
        if !dir_seen[index].is_empty() {
            dense_count(dir_seen[index].iter().copied())?;
            if step.directories.len() > MAXIMUM_DIRECTORIES {
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
    if index >= MAXIMUM_DIRECTORIES.max(MAXIMUM_OUTPUTS) {
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

fn empty_step() -> StepDraft {
    StepDraft {
        key: String::new(),
        name: String::new(),
        action: "agent".to_owned(),
        role: String::new(),
        command: SystemCommandId::RepositoryStatus.as_str().to_owned(),
        tools: Vec::new(),
        directories: Vec::new(),
        outputs: Vec::new(),
    }
}

fn blank_agent_step(key: &str, role: &str) -> StepDraft {
    let mut directories = vec![DirectoryDraft {
        alias: "project".to_owned(),
        access: AccessMode::ReadWrite.as_str().to_owned(),
    }];
    pad_directories(&mut directories);
    StepDraft {
        key: key.to_owned(),
        name: String::new(),
        action: "agent".to_owned(),
        role: role.to_owned(),
        command: SystemCommandId::RepositoryStatus.as_str().to_owned(),
        tools: ToolId::ALL.to_vec(),
        directories,
        outputs: vec![OutputDraft {
            key: "assistant-reply".to_owned(),
            kind: OutputKind::AssistantReply.as_str().to_owned(),
        }],
    }
}

fn pad_directories(directories: &mut Vec<DirectoryDraft>) {
    while directories.len() < MAXIMUM_DIRECTORIES {
        directories.push(DirectoryDraft {
            alias: String::new(),
            access: AccessMode::ReadWrite.as_str().to_owned(),
        });
    }
}

fn step_from_definition(step: &StepDefinition) -> StepDraft {
    match &step.action {
        StepAction::Agent(action) => {
            let mut directories: Vec<DirectoryDraft> = action
                .authority
                .directories
                .iter()
                .map(|directory| DirectoryDraft {
                    alias: directory.alias.clone(),
                    access: directory.access.as_str().to_owned(),
                })
                .collect();
            pad_directories(&mut directories);
            StepDraft {
                key: step.key.as_str().to_owned(),
                name: step.name.clone(),
                action: "agent".to_owned(),
                role: action.role.as_str().to_owned(),
                command: SystemCommandId::RepositoryStatus.as_str().to_owned(),
                tools: action.authority.tools.clone(),
                directories,
                outputs: action
                    .required_outputs
                    .iter()
                    .map(|output| OutputDraft {
                        key: output.key.as_str().to_owned(),
                        kind: output.kind.as_str().to_owned(),
                    })
                    .collect(),
            }
        }
        StepAction::SystemCommand(action) => StepDraft {
            key: step.key.as_str().to_owned(),
            name: step.name.clone(),
            action: "system-command".to_owned(),
            role: String::new(),
            command: action.command.as_str().to_owned(),
            tools: Vec::new(),
            directories: Vec::new(),
            outputs: Vec::new(),
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
        _ => {
            errors.action = "Choose an action type.";
            return None;
        }
    };
    Some(StepDefinition {
        key,
        name: display_name,
        action,
        on_success: SuccessTransition::CompleteRun,
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
    let mut directories = Vec::new();
    for (index, directory) in step.directories.iter().enumerate() {
        if directory.alias.trim().is_empty() {
            continue;
        }
        let Some(access) = AccessMode::parse(&directory.access) else {
            errors.directories[index].alias =
                crate::workflows::definition::DefinitionError::Format.message();
            return None;
        };
        directories.push(GuestDirectoryAccess {
            alias: directory.alias.clone(),
            access,
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
    Some(StepAction::Agent(AgentStep {
        role,
        authority,
        required_outputs: outputs,
    }))
}

fn build_command_action(step: &StepDraft, errors: &mut StepErrors) -> Option<StepAction> {
    let Some(command) = SystemCommandId::parse(&step.command) else {
        errors.command = crate::workflows::definition::DefinitionError::Command.message();
        return None;
    };
    Some(StepAction::SystemCommand(SystemCommandStep {
        command,
        required_outputs: Vec::new(),
    }))
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
        _ => {}
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

fn move_item<T>(items: &mut [T], index: usize, up: bool) -> Result<(), FormError> {
    if index >= items.len() {
        return Err(FormError::Index);
    }
    let target = if up {
        index.checked_sub(1).ok_or(FormError::Index)?
    } else {
        let next = index.checked_add(1).ok_or(FormError::Index)?;
        if next >= items.len() {
            return Err(FormError::Index);
        }
        next
    };
    items.swap(index, target);
    Ok(())
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
