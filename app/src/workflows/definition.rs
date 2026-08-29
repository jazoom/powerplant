use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agents::{AccessMode, ToolId};

use super::id::WorkflowId;

pub(crate) const DEFINITION_FORMAT_VERSION: u32 = 1;
pub(crate) const MAXIMUM_NAME_BYTES: usize = 80;
pub(crate) const MAXIMUM_EXPERTISE_BYTES: usize = 4_096;
pub(crate) const MAXIMUM_PROMPT_DEFAULTS_BYTES: usize = 32_768;
pub(crate) const MAXIMUM_KEY_BYTES: usize = 32;
pub(crate) const MAXIMUM_ROLES: usize = 16;
pub(crate) const MAXIMUM_STEPS: usize = 32;
pub(crate) const MAXIMUM_OUTPUTS: usize = 8;
pub(crate) const MAXIMUM_DIRECTORIES: usize = 8;
pub(crate) const ASSISTANT_REPLY: &str = "assistant-reply";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkflowDefinition {
    format_version: u32,
    name: String,
    roles: Vec<RoleDefinition>,
    first_step: StepKey,
    steps: Vec<StepDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RoleDefinition {
    pub(crate) key: RoleKey,
    pub(crate) name: String,
    pub(crate) expertise: String,
    pub(crate) prompt_defaults: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StepDefinition {
    pub(crate) key: StepKey,
    pub(crate) name: String,
    pub(crate) action: StepAction,
    pub(crate) on_success: SuccessTransition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StepAction {
    Agent(AgentStep),
    SystemCommand(SystemCommandStep),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentStep {
    pub(crate) role: RoleKey,
    pub(crate) authority: AgentAuthority,
    pub(crate) required_outputs: Vec<RequiredOutput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SystemCommandStep {
    pub(crate) command: SystemCommandId,
    pub(crate) required_outputs: Vec<RequiredOutput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentAuthority {
    pub(crate) tools: Vec<ToolId>,
    pub(crate) directories: Vec<GuestDirectoryAccess>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GuestDirectoryAccess {
    pub(crate) alias: String,
    pub(crate) access: AccessMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RequiredOutput {
    pub(crate) key: OutputKey,
    pub(crate) kind: OutputKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutputKind {
    AssistantReply,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SuccessTransition {
    Next(StepKey),
    CompleteRun,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SystemCommandId {
    RepositoryStatus,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RoleKey(String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct StepKey(String);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct OutputKey(String);

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct DefinitionVersion([u8; 32]);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PinnedWorkflowDefinition {
    pub(crate) workflow_id: Option<WorkflowId>,
    pub(crate) version: DefinitionVersion,
    pub(crate) definition: WorkflowDefinition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DefinitionError {
    Format,
    Name,
    Expertise,
    PromptDefaults,
    Key,
    RoleCount,
    StepCount,
    OutputCount,
    DirectoryCount,
    DuplicateRole,
    DuplicateStep,
    DuplicateOutput,
    UnsupportedOutput,
    UnusedRole,
    UnknownRole,
    UnknownStep,
    Cycle,
    Branch,
    Join,
    Unreachable,
    Command,
    Tools,
    Alias,
    DuplicateAlias,
}

impl DefinitionError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Format => "That workflow definition uses an unsupported format.",
            Self::Name => "Enter a name of at most 80 bytes.",
            Self::Expertise => "Those expertise notes are too long.",
            Self::PromptDefaults => "Those prompt defaults are too long.",
            Self::Key => "Enter a key that uses letters, numbers, hyphen or underscore.",
            Self::RoleCount => "Add at most 16 roles.",
            Self::StepCount => "Add between one and 32 steps.",
            Self::OutputCount => "Add at most eight required outputs for each step.",
            Self::DirectoryCount => "Add between one and eight directory grants.",
            Self::DuplicateRole => "Role keys must be unique.",
            Self::DuplicateStep => "Step keys must be unique.",
            Self::DuplicateOutput => "Output keys must be unique in a step.",
            Self::UnsupportedOutput => "That action cannot produce the required output.",
            Self::UnusedRole => "Every role must be used by an agent step.",
            Self::UnknownRole => "An agent step names an unknown role.",
            Self::UnknownStep => "A step names an unknown successor.",
            Self::Cycle => "The workflow graph cannot contain a cycle.",
            Self::Branch => "This step only supports one serial chain.",
            Self::Join => "This step only supports one serial chain.",
            Self::Unreachable => "Every step must sit on the serial chain.",
            Self::Command => "Choose a registered system command.",
            Self::Tools => "Choose tools from the built-in set.",
            Self::Alias => {
                "Enter a directory alias that uses letters, numbers, hyphen or underscore."
            }
            Self::DuplicateAlias => "Directory aliases must be unique.",
        }
    }
}

impl std::fmt::Display for DefinitionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for DefinitionError {}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct DefinitionFile {
    format_version: u32,
    name: String,
    roles: Vec<RoleFile>,
    first_step: String,
    steps: Vec<StepFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct RoleFile {
    key: String,
    name: String,
    expertise: String,
    prompt_defaults: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct StepFile {
    key: String,
    name: String,
    action: ActionFile,
    on_success: TransitionFile,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum ActionFile {
    Agent {
        role: String,
        authority: AuthorityFile,
        #[serde(rename = "required-outputs")]
        required_outputs: Vec<OutputFile>,
    },
    SystemCommand {
        command: String,
        #[serde(rename = "required-outputs")]
        required_outputs: Vec<OutputFile>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct AuthorityFile {
    tools: Vec<String>,
    directories: Vec<DirectoryFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct DirectoryFile {
    alias: String,
    access: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct OutputFile {
    key: String,
    kind: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", content = "step", rename_all = "kebab-case")]
enum TransitionFile {
    Next(String),
    CompleteRun,
}

impl WorkflowDefinition {
    #[cfg(test)]
    pub(crate) fn from_file_bytes(bytes: &[u8]) -> Result<Self, DefinitionError> {
        let file: DefinitionFile =
            serde_json::from_slice(bytes).map_err(|_| DefinitionError::Format)?;
        Self::from_file(file)
    }

    pub(super) fn from_file(file: DefinitionFile) -> Result<Self, DefinitionError> {
        if file.format_version != DEFINITION_FORMAT_VERSION {
            return Err(DefinitionError::Format);
        }
        let name = normalise_name(&file.name)?;
        if file.roles.len() > MAXIMUM_ROLES {
            return Err(DefinitionError::RoleCount);
        }
        if file.steps.is_empty() || file.steps.len() > MAXIMUM_STEPS {
            return Err(DefinitionError::StepCount);
        }
        let roles = file
            .roles
            .into_iter()
            .map(RoleDefinition::from_file)
            .collect::<Result<Vec<_>, _>>()?;
        let first_step = StepKey::parse(&file.first_step)?;
        let steps = file
            .steps
            .into_iter()
            .map(StepDefinition::from_file)
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_parts(name, roles, first_step, steps)
    }

    pub(crate) fn from_parts(
        name: String,
        roles: Vec<RoleDefinition>,
        first_step: StepKey,
        steps: Vec<StepDefinition>,
    ) -> Result<Self, DefinitionError> {
        let name = normalise_name(&name)?;
        if roles.len() > MAXIMUM_ROLES {
            return Err(DefinitionError::RoleCount);
        }
        if steps.is_empty() || steps.len() > MAXIMUM_STEPS {
            return Err(DefinitionError::StepCount);
        }
        reject_duplicate_roles(&roles)?;
        reject_duplicate_steps(&steps)?;
        reject_step_outputs(&steps)?;
        reject_unsupported_outputs(&steps)?;
        reject_role_use(&roles, &steps)?;
        validate_serial_chain(&first_step, &steps)?;
        Ok(Self {
            format_version: DEFINITION_FORMAT_VERSION,
            name,
            roles,
            first_step,
            steps,
        })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    #[cfg(test)]
    pub(crate) fn roles(&self) -> &[RoleDefinition] {
        &self.roles
    }

    pub(crate) fn first_step(&self) -> &StepKey {
        &self.first_step
    }

    pub(crate) fn steps(&self) -> &[StepDefinition] {
        &self.steps
    }

    pub(crate) fn step(&self, key: &StepKey) -> Option<&StepDefinition> {
        self.steps.iter().find(|step| step.key == *key)
    }

    pub(crate) fn role(&self, key: &RoleKey) -> Option<&RoleDefinition> {
        self.roles.iter().find(|role| role.key == *key)
    }

    pub(crate) fn version(&self) -> DefinitionVersion {
        DefinitionVersion::of(&self.to_file())
    }

    pub(crate) fn to_file(&self) -> DefinitionFile {
        DefinitionFile {
            format_version: self.format_version,
            name: self.name.clone(),
            roles: self
                .roles
                .iter()
                .map(|role| RoleFile {
                    key: role.key.as_str().to_owned(),
                    name: role.name.clone(),
                    expertise: role.expertise.clone(),
                    prompt_defaults: role.prompt_defaults.clone(),
                })
                .collect(),
            first_step: self.first_step.as_str().to_owned(),
            steps: self.steps.iter().map(StepDefinition::to_file).collect(),
        }
    }
}

impl RoleDefinition {
    pub(crate) fn new(
        key: RoleKey,
        name: String,
        expertise: String,
        prompt_defaults: String,
    ) -> Result<Self, DefinitionError> {
        Ok(Self {
            key,
            name: normalise_name(&name)?,
            expertise: normalise_text(
                &expertise,
                MAXIMUM_EXPERTISE_BYTES,
                DefinitionError::Expertise,
            )?,
            prompt_defaults: normalise_text(
                &prompt_defaults,
                MAXIMUM_PROMPT_DEFAULTS_BYTES,
                DefinitionError::PromptDefaults,
            )?,
        })
    }

    fn from_file(file: RoleFile) -> Result<Self, DefinitionError> {
        Self::new(
            RoleKey::parse(&file.key)?,
            file.name,
            file.expertise,
            file.prompt_defaults,
        )
    }
}

impl StepDefinition {
    fn from_file(file: StepFile) -> Result<Self, DefinitionError> {
        Ok(Self {
            key: StepKey::parse(&file.key)?,
            name: normalise_name(&file.name)?,
            action: StepAction::from_file(file.action)?,
            on_success: SuccessTransition::from_file(file.on_success)?,
        })
    }

    fn to_file(&self) -> StepFile {
        StepFile {
            key: self.key.as_str().to_owned(),
            name: self.name.clone(),
            action: self.action.to_file(),
            on_success: self.on_success.to_file(),
        }
    }
}

impl StepAction {
    fn from_file(file: ActionFile) -> Result<Self, DefinitionError> {
        match file {
            ActionFile::Agent {
                role,
                authority,
                required_outputs,
            } => Ok(Self::Agent(AgentStep {
                role: RoleKey::parse(&role)?,
                authority: AgentAuthority::from_file(authority)?,
                required_outputs: parse_outputs(required_outputs)?,
            })),
            ActionFile::SystemCommand {
                command,
                required_outputs,
            } => Ok(Self::SystemCommand(SystemCommandStep {
                command: SystemCommandId::parse(&command).ok_or(DefinitionError::Command)?,
                required_outputs: parse_outputs(required_outputs)?,
            })),
        }
    }

    fn to_file(&self) -> ActionFile {
        match self {
            Self::Agent(step) => ActionFile::Agent {
                role: step.role.as_str().to_owned(),
                authority: step.authority.to_file(),
                required_outputs: outputs_to_file(&step.required_outputs),
            },
            Self::SystemCommand(step) => ActionFile::SystemCommand {
                command: step.command.as_str().to_owned(),
                required_outputs: outputs_to_file(&step.required_outputs),
            },
        }
    }

    pub(crate) fn kind_label(&self) -> &'static str {
        match self {
            Self::Agent(_) => "Agent",
            Self::SystemCommand(_) => "System command",
        }
    }
}

impl AgentAuthority {
    pub(crate) fn new(
        tools: Vec<ToolId>,
        directories: Vec<GuestDirectoryAccess>,
    ) -> Result<Self, DefinitionError> {
        if tools.len() > ToolId::ALL.len() {
            return Err(DefinitionError::Tools);
        }
        let mut unique_tools = Vec::new();
        for tool in tools {
            if unique_tools.contains(&tool) {
                return Err(DefinitionError::Tools);
            }
            unique_tools.push(tool);
        }
        let tools = ToolId::ALL
            .into_iter()
            .filter(|tool| unique_tools.contains(tool))
            .collect();
        if directories.is_empty() || directories.len() > MAXIMUM_DIRECTORIES {
            return Err(DefinitionError::DirectoryCount);
        }
        let mut unique_dirs = Vec::new();
        for directory in directories {
            let alias = normalise_alias(&directory.alias)?;
            if unique_dirs
                .iter()
                .any(|seen: &GuestDirectoryAccess| seen.alias == alias)
            {
                return Err(DefinitionError::DuplicateAlias);
            }
            unique_dirs.push(GuestDirectoryAccess {
                alias,
                access: directory.access,
            });
        }
        Ok(Self {
            tools,
            directories: unique_dirs,
        })
    }

    fn from_file(file: AuthorityFile) -> Result<Self, DefinitionError> {
        let mut tools = Vec::new();
        for name in file.tools {
            let tool = ToolId::parse(&name).ok_or(DefinitionError::Tools)?;
            tools.push(tool);
        }
        let directories = file
            .directories
            .into_iter()
            .map(|directory| {
                let access = AccessMode::parse(&directory.access).ok_or(DefinitionError::Format)?;
                Ok(GuestDirectoryAccess {
                    alias: directory.alias,
                    access,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(tools, directories)
    }

    fn to_file(&self) -> AuthorityFile {
        AuthorityFile {
            tools: self
                .tools
                .iter()
                .map(|tool| tool.as_str().to_owned())
                .collect(),
            directories: self
                .directories
                .iter()
                .map(|directory| DirectoryFile {
                    alias: directory.alias.clone(),
                    access: directory.access.as_str().to_owned(),
                })
                .collect(),
        }
    }
}

impl SuccessTransition {
    fn from_file(file: TransitionFile) -> Result<Self, DefinitionError> {
        match file {
            TransitionFile::Next(step) => Ok(Self::Next(StepKey::parse(&step)?)),
            TransitionFile::CompleteRun => Ok(Self::CompleteRun),
        }
    }

    fn to_file(&self) -> TransitionFile {
        match self {
            Self::Next(step) => TransitionFile::Next(step.as_str().to_owned()),
            Self::CompleteRun => TransitionFile::CompleteRun,
        }
    }
}

impl SystemCommandId {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "repository-status" => Some(Self::RepositoryStatus),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RepositoryStatus => "repository-status",
        }
    }
}

impl OutputKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "assistant-reply" => Some(Self::AssistantReply),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::AssistantReply => "assistant-reply",
        }
    }
}

impl RoleKey {
    pub(crate) fn parse(value: &str) -> Result<Self, DefinitionError> {
        Ok(Self(parse_key(value)?))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl StepKey {
    pub(crate) fn parse(value: &str) -> Result<Self, DefinitionError> {
        Ok(Self(parse_key(value)?))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl OutputKey {
    pub(crate) fn parse(value: &str) -> Result<Self, DefinitionError> {
        Ok(Self(parse_key(value)?))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl DefinitionVersion {
    fn of(file: &DefinitionFile) -> Self {
        let bytes = serde_json::to_vec(file).expect("canonical definition json");
        Self(Sha256::digest(bytes).into())
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = decode_hex_byte(chunk[0], chunk[1])?;
        }
        Some(Self(bytes))
    }

    pub(crate) fn as_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }
}

impl std::fmt::Debug for DefinitionVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DefinitionVersion(")?;
        formatter.write_str(&self.as_hex())?;
        formatter.write_str(")")
    }
}

impl PinnedWorkflowDefinition {
    pub(crate) fn pin(workflow_id: Option<WorkflowId>, definition: WorkflowDefinition) -> Self {
        let version = definition.version();
        Self {
            workflow_id,
            version,
            definition,
        }
    }
}

fn parse_outputs(files: Vec<OutputFile>) -> Result<Vec<RequiredOutput>, DefinitionError> {
    if files.len() > MAXIMUM_OUTPUTS {
        return Err(DefinitionError::OutputCount);
    }
    let mut outputs = Vec::with_capacity(files.len());
    for file in files {
        let key = OutputKey::parse(&file.key)?;
        if outputs.iter().any(|item: &RequiredOutput| item.key == key) {
            return Err(DefinitionError::DuplicateOutput);
        }
        let kind = OutputKind::parse(&file.kind).ok_or(DefinitionError::Format)?;
        outputs.push(RequiredOutput { key, kind });
    }
    Ok(outputs)
}

fn outputs_to_file(outputs: &[RequiredOutput]) -> Vec<OutputFile> {
    outputs
        .iter()
        .map(|output| OutputFile {
            key: output.key.as_str().to_owned(),
            kind: output.kind.as_str().to_owned(),
        })
        .collect()
}

fn reject_duplicate_roles(roles: &[RoleDefinition]) -> Result<(), DefinitionError> {
    for (index, role) in roles.iter().enumerate() {
        if roles
            .iter()
            .skip(index + 1)
            .any(|other| other.key == role.key)
        {
            return Err(DefinitionError::DuplicateRole);
        }
    }
    Ok(())
}

fn reject_duplicate_steps(steps: &[StepDefinition]) -> Result<(), DefinitionError> {
    for (index, step) in steps.iter().enumerate() {
        if steps
            .iter()
            .skip(index + 1)
            .any(|other| other.key == step.key)
        {
            return Err(DefinitionError::DuplicateStep);
        }
    }
    Ok(())
}

fn reject_unsupported_outputs(steps: &[StepDefinition]) -> Result<(), DefinitionError> {
    for step in steps {
        if let StepAction::SystemCommand(action) = &step.action
            && !action.required_outputs.is_empty()
        {
            return Err(DefinitionError::UnsupportedOutput);
        }
    }
    Ok(())
}

fn reject_role_use(
    roles: &[RoleDefinition],
    steps: &[StepDefinition],
) -> Result<(), DefinitionError> {
    let mut used = Vec::new();
    for step in steps {
        if let StepAction::Agent(action) = &step.action {
            if !roles.iter().any(|role| role.key == action.role) {
                return Err(DefinitionError::UnknownRole);
            }
            if !used.contains(&action.role) {
                used.push(action.role.clone());
            }
        }
    }
    if used.len() != roles.len() {
        return Err(DefinitionError::UnusedRole);
    }
    Ok(())
}

fn reject_step_outputs(steps: &[StepDefinition]) -> Result<(), DefinitionError> {
    for step in steps {
        let outputs = match &step.action {
            StepAction::Agent(action) => &action.required_outputs,
            StepAction::SystemCommand(action) => &action.required_outputs,
        };
        if outputs.len() > MAXIMUM_OUTPUTS {
            return Err(DefinitionError::OutputCount);
        }
        for (index, output) in outputs.iter().enumerate() {
            if outputs
                .iter()
                .skip(index + 1)
                .any(|other| other.key == output.key)
            {
                return Err(DefinitionError::DuplicateOutput);
            }
        }
    }
    Ok(())
}

fn validate_serial_chain(first: &StepKey, steps: &[StepDefinition]) -> Result<(), DefinitionError> {
    if !steps.iter().any(|step| step.key == *first) {
        return Err(DefinitionError::UnknownStep);
    }
    let mut indegree = vec![0u32; steps.len()];
    for step in steps {
        if let SuccessTransition::Next(successor) = &step.on_success {
            let Some(index) = steps.iter().position(|item| item.key == *successor) else {
                return Err(DefinitionError::UnknownStep);
            };
            indegree[index] = indegree[index]
                .checked_add(1)
                .ok_or(DefinitionError::Join)?;
            if indegree[index] > 1 {
                return Err(DefinitionError::Join);
            }
        }
    }
    let mut visited = Vec::new();
    let mut current = first.clone();
    loop {
        if visited.contains(&current) {
            return Err(DefinitionError::Cycle);
        }
        let step = steps
            .iter()
            .find(|item| item.key == current)
            .ok_or(DefinitionError::UnknownStep)?;
        visited.push(current);
        match &step.on_success {
            SuccessTransition::Next(next) => current = next.clone(),
            SuccessTransition::CompleteRun => break,
        }
    }
    if visited.len() != steps.len() {
        return Err(DefinitionError::Unreachable);
    }
    let sources = indegree.iter().filter(|count| **count == 0).count();
    if sources != 1 {
        return Err(DefinitionError::Branch);
    }
    Ok(())
}

fn parse_key(raw: &str) -> Result<String, DefinitionError> {
    let key = raw.trim();
    if key.is_empty() || key.len() > MAXIMUM_KEY_BYTES {
        return Err(DefinitionError::Key);
    }
    let mut characters = key.chars();
    let Some(first) = characters.next() else {
        return Err(DefinitionError::Key);
    };
    if !first.is_ascii_alphabetic() {
        return Err(DefinitionError::Key);
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
    {
        return Err(DefinitionError::Key);
    }
    Ok(key.to_owned())
}

fn normalise_name(raw: &str) -> Result<String, DefinitionError> {
    let name = raw.trim();
    if name.is_empty() || name.len() > MAXIMUM_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(DefinitionError::Name);
    }
    Ok(name.to_owned())
}

fn normalise_text(
    raw: &str,
    maximum: usize,
    error: DefinitionError,
) -> Result<String, DefinitionError> {
    if raw.len() > maximum
        || raw
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(error);
    }
    Ok(raw.to_owned())
}

fn normalise_alias(raw: &str) -> Result<String, DefinitionError> {
    let alias = parse_key(raw).map_err(|_| DefinitionError::Alias)?;
    Ok(alias)
}

fn decode_hex_byte(high: u8, low: u8) -> Option<u8> {
    Some((decode_hex_nibble(high)? << 4) | decode_hex_nibble(low)?)
}

fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
