use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agents::{AccessMode, ToolId};
use crate::environments::EnvironmentId;

use super::commands::{CommandSourceEffect, kinds_match};
use super::id::WorkflowId;

pub(crate) use super::commands::SystemCommandId;

pub(crate) const DEFINITION_FORMAT_VERSION: u32 = 2;
pub(crate) const MAXIMUM_NAME_BYTES: usize = 80;
pub(crate) const MAXIMUM_EXPERTISE_BYTES: usize = 4_096;
pub(crate) const MAXIMUM_PROMPT_DEFAULTS_BYTES: usize = 32_768;
pub(crate) const MAXIMUM_KEY_BYTES: usize = 32;
pub(crate) const MAXIMUM_ROLES: usize = 16;
pub(crate) const MAXIMUM_STEPS: usize = 32;
pub(crate) const MAXIMUM_OUTPUTS: usize = 8;
pub(crate) const MAXIMUM_INPUTS: usize = 8;
pub(crate) const MAXIMUM_DIRECTORIES: usize = 8;
pub(crate) const ASSISTANT_REPLY: &str = "assistant-reply";
pub(crate) const PRIMARY_SOURCE_ALIAS: &str = "project";
pub(crate) const CANDIDATE_OUTPUT_KEY: &str = "candidate";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkflowDefinition {
    format_version: u32,
    name: String,
    default_environment: EnvironmentId,
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
    pub(crate) inputs: Vec<RequiredInput>,
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
    pub(crate) environment: StepEnvironment,
    pub(crate) candidate_authority: CandidateAuthority,
    pub(crate) authority: AgentAuthority,
    pub(crate) required_outputs: Vec<RequiredOutput>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CandidateAuthority {
    ReadOnly,
    Edit,
}

impl CandidateAuthority {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "read-only" => Some(Self::ReadOnly),
            "edit-candidate" => Some(Self::Edit),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Edit => "edit-candidate",
        }
    }

    pub(crate) fn access(self) -> AccessMode {
        match self {
            Self::ReadOnly => AccessMode::ReadOnly,
            Self::Edit => AccessMode::ReadWrite,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read-only",
            Self::Edit => "Can edit candidate",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SystemCommandStep {
    pub(crate) command: SystemCommandId,
    pub(crate) environment: StepEnvironment,
    pub(crate) required_outputs: Vec<RequiredOutput>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StepEnvironment {
    WorkflowDefault,
    Override { environment_id: EnvironmentId },
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
pub(crate) struct RequiredInput {
    pub(crate) key: InputKey,
    pub(crate) kind: ArtefactKind,
    pub(crate) source: ArtefactSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ArtefactSource {
    RunInitialCandidate,
    StepOutput { step: StepKey, output: OutputKey },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RequiredOutput {
    pub(crate) key: OutputKey,
    pub(crate) kind: OutputKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutputKind {
    AssistantReply,
    Plan,
    CandidateRevision,
    ReviewReport,
    TestReport,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ArtefactKind {
    Plan,
    CandidateRevision,
    ReviewReport,
    TestReport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SuccessTransition {
    Next(StepKey),
    CompleteRun,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RoleKey(String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct StepKey(String);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct OutputKey(String);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct InputKey(String);

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
    Environment,
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
    DuplicateInput,
    InputCount,
    UnsupportedOutput,
    ForwardInput,
    SelfInput,
    UnknownOutput,
    InputKind,
    AssistantInput,
    CandidateInput,
    CandidateOutput,
    AssuranceInput,
    SecondaryWrite,
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
            Self::Environment => "Enter a valid environment identifier.",
            Self::Name => "Enter a name of at most 80 bytes.",
            Self::Expertise => "Those expertise notes are too long.",
            Self::PromptDefaults => "Those prompt defaults are too long.",
            Self::Key => "Enter a key that uses letters, numbers, hyphen or underscore.",
            Self::RoleCount => "Add at most 16 roles.",
            Self::StepCount => "Add between one and 32 steps.",
            Self::OutputCount => "Add at most eight required outputs for each step.",
            Self::DirectoryCount => "Add at most eight secondary directory grants.",
            Self::DuplicateRole => "Role keys must be unique.",
            Self::DuplicateStep => "Step keys must be unique.",
            Self::DuplicateOutput => "Output keys must be unique in a step.",
            Self::DuplicateInput => "Input keys must be unique in a step.",
            Self::InputCount => "Add at most eight inputs for each step.",
            Self::UnsupportedOutput => "That action cannot produce the required output.",
            Self::ForwardInput => "An input cannot name a later step.",
            Self::SelfInput => "An input cannot name its own step.",
            Self::UnknownOutput => "An input names an unknown earlier output.",
            Self::InputKind => "That input kind does not match the named output.",
            Self::AssistantInput => "Assistant replies cannot be artefact inputs.",
            Self::CandidateInput => {
                "Each sandbox-backed step needs exactly one latest candidate input."
            }
            Self::CandidateOutput => {
                "Candidate access does not match the candidate revision outputs."
            }
            Self::AssuranceInput => {
                "A step that uses an assurance artefact also needs a candidate input."
            }
            Self::SecondaryWrite => "Secondary directory grants must stay read-only.",
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct DefinitionFile {
    format_version: u32,
    name: String,
    default_environment: String,
    roles: Vec<RoleFile>,
    first_step: String,
    steps: Vec<StepFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct RoleFile {
    key: String,
    name: String,
    expertise: String,
    prompt_defaults: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct StepFile {
    key: String,
    name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    inputs: Vec<InputFile>,
    action: ActionFile,
    on_success: TransitionFile,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum ActionFile {
    Agent {
        role: String,
        environment: StepEnvironmentFile,
        #[serde(rename = "candidate-authority")]
        candidate_authority: String,
        authority: AuthorityFile,
        #[serde(rename = "required-outputs")]
        required_outputs: Vec<OutputFile>,
    },
    SystemCommand {
        command: String,
        environment: StepEnvironmentFile,
        #[serde(rename = "required-outputs")]
        required_outputs: Vec<OutputFile>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "kebab-case")]
enum StepEnvironmentFile {
    WorkflowDefault,
    Override { environment_id: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct AuthorityFile {
    tools: Vec<String>,
    directories: Vec<DirectoryFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct DirectoryFile {
    alias: String,
    access: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct OutputFile {
    key: String,
    kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct InputFile {
    key: String,
    kind: String,
    source: InputSourceFile,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "kebab-case")]
enum InputSourceFile {
    RunInitialCandidate,
    StepOutput { step: String, output: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

    pub(crate) fn from_file(file: DefinitionFile) -> Result<Self, DefinitionError> {
        if file.format_version != DEFINITION_FORMAT_VERSION {
            return Err(DefinitionError::Format);
        }
        from_current_file(file)
    }

    pub(crate) fn from_parts(
        name: String,
        default_environment: EnvironmentId,
        roles: Vec<RoleDefinition>,
        first_step: StepKey,
        steps: Vec<StepDefinition>,
    ) -> Result<Self, DefinitionError> {
        assemble(name, default_environment, roles, first_step, steps)
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn default_environment(&self) -> EnvironmentId {
        self.default_environment
    }

    pub(crate) fn referenced_environments(&self) -> Vec<EnvironmentId> {
        let mut ids = vec![self.default_environment];
        for step in &self.steps {
            if let StepEnvironment::Override { environment_id } = step.environment()
                && !ids.contains(&environment_id)
            {
                ids.push(environment_id);
            }
        }
        ids
    }

    pub(crate) fn effective_environment(&self, step: &StepDefinition) -> EnvironmentId {
        match step.environment() {
            StepEnvironment::WorkflowDefault => self.default_environment,
            StepEnvironment::Override { environment_id } => environment_id,
        }
    }

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
            default_environment: self.default_environment.as_hex(),
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
            inputs: parse_inputs(file.inputs)?,
            action: StepAction::from_file(file.action)?,
            on_success: SuccessTransition::from_file(file.on_success)?,
        })
    }

    pub(crate) fn environment(&self) -> StepEnvironment {
        match &self.action {
            StepAction::Agent(action) => action.environment,
            StepAction::SystemCommand(action) => action.environment,
        }
    }

    pub(crate) fn is_sandbox_backed(&self) -> bool {
        matches!(
            self.action,
            StepAction::Agent(_) | StepAction::SystemCommand(_)
        )
    }

    pub(crate) fn required_outputs(&self) -> &[RequiredOutput] {
        match &self.action {
            StepAction::Agent(action) => &action.required_outputs,
            StepAction::SystemCommand(action) => &action.required_outputs,
        }
    }

    pub(crate) fn writes_primary_source(&self) -> bool {
        matches!(
            &self.action,
            StepAction::Agent(AgentStep {
                candidate_authority: CandidateAuthority::Edit,
                ..
            })
        )
    }

    pub(crate) fn command_source_effect(&self) -> Option<CommandSourceEffect> {
        match &self.action {
            StepAction::SystemCommand(action) => Some(action.command.contract().source_effect),
            StepAction::Agent(_) => None,
        }
    }

    fn to_file(&self) -> StepFile {
        StepFile {
            key: self.key.as_str().to_owned(),
            name: self.name.clone(),
            inputs: self
                .inputs
                .iter()
                .map(|input| InputFile {
                    key: input.key.as_str().to_owned(),
                    kind: input.kind.as_str().to_owned(),
                    source: match &input.source {
                        ArtefactSource::RunInitialCandidate => InputSourceFile::RunInitialCandidate,
                        ArtefactSource::StepOutput { step, output } => {
                            InputSourceFile::StepOutput {
                                step: step.as_str().to_owned(),
                                output: output.as_str().to_owned(),
                            }
                        }
                    },
                })
                .collect(),
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
                environment,
                candidate_authority,
                authority,
                required_outputs,
            } => Ok(Self::Agent(AgentStep {
                role: RoleKey::parse(&role)?,
                environment: StepEnvironment::from_file(environment)?,
                candidate_authority: CandidateAuthority::parse(&candidate_authority)
                    .ok_or(DefinitionError::Format)?,
                authority: AgentAuthority::from_file(authority)?,
                required_outputs: parse_outputs(required_outputs)?,
            })),
            ActionFile::SystemCommand {
                command,
                environment,
                required_outputs,
            } => Ok(Self::SystemCommand(SystemCommandStep {
                command: SystemCommandId::parse(&command).ok_or(DefinitionError::Command)?,
                environment: StepEnvironment::from_file(environment)?,
                required_outputs: parse_outputs(required_outputs)?,
            })),
        }
    }

    fn to_file(&self) -> ActionFile {
        match self {
            Self::Agent(step) => ActionFile::Agent {
                role: step.role.as_str().to_owned(),
                environment: step.environment.to_file(),
                candidate_authority: step.candidate_authority.as_str().to_owned(),
                authority: step.authority.to_file(),
                required_outputs: outputs_to_file(&step.required_outputs),
            },
            Self::SystemCommand(step) => ActionFile::SystemCommand {
                command: step.command.as_str().to_owned(),
                environment: step.environment.to_file(),
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
        if directories.len() > MAXIMUM_DIRECTORIES {
            return Err(DefinitionError::DirectoryCount);
        }
        let mut unique_dirs = Vec::new();
        for directory in directories {
            let alias = normalise_alias(&directory.alias)?;
            if alias == PRIMARY_SOURCE_ALIAS {
                return Err(DefinitionError::Alias);
            }
            if directory.access.is_writable() {
                return Err(DefinitionError::SecondaryWrite);
            }
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

    pub(crate) fn allowed_by<'a>(
        &self,
        tools: &[ToolId],
        directories: impl IntoIterator<Item = (&'a str, AccessMode)>,
    ) -> bool {
        let grants: Vec<(&str, AccessMode)> = directories.into_iter().collect();
        self.tools.iter().all(|tool| tools.contains(tool))
            && self.directories.iter().all(|directory| {
                grants.iter().any(|(alias, access)| {
                    *alias == directory.alias
                        && (!directory.access.is_writable() || access.is_writable())
                })
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

impl StepEnvironment {
    fn from_file(file: StepEnvironmentFile) -> Result<Self, DefinitionError> {
        match file {
            StepEnvironmentFile::WorkflowDefault => Ok(Self::WorkflowDefault),
            StepEnvironmentFile::Override { environment_id } => {
                let environment_id =
                    EnvironmentId::parse(&environment_id).ok_or(DefinitionError::Environment)?;
                Ok(Self::Override { environment_id })
            }
        }
    }

    fn to_file(self) -> StepEnvironmentFile {
        match self {
            Self::WorkflowDefault => StepEnvironmentFile::WorkflowDefault,
            Self::Override { environment_id } => StepEnvironmentFile::Override {
                environment_id: environment_id.as_hex(),
            },
        }
    }

    fn normalised(self, default: EnvironmentId) -> Self {
        match self {
            Self::Override { environment_id } if environment_id == default => Self::WorkflowDefault,
            other => other,
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

impl OutputKind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "assistant-reply" => Some(Self::AssistantReply),
            "plan" => Some(Self::Plan),
            "candidate-revision" => Some(Self::CandidateRevision),
            "review-report" => Some(Self::ReviewReport),
            "test-report" => Some(Self::TestReport),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AssistantReply => "assistant-reply",
            Self::Plan => "plan",
            Self::CandidateRevision => "candidate-revision",
            Self::ReviewReport => "review-report",
            Self::TestReport => "test-report",
        }
    }

    pub(crate) fn as_artefact_kind(self) -> Option<ArtefactKind> {
        match self {
            Self::AssistantReply => None,
            Self::Plan => Some(ArtefactKind::Plan),
            Self::CandidateRevision => Some(ArtefactKind::CandidateRevision),
            Self::ReviewReport => Some(ArtefactKind::ReviewReport),
            Self::TestReport => Some(ArtefactKind::TestReport),
        }
    }
}

impl ArtefactKind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "plan" => Some(Self::Plan),
            "candidate-revision" => Some(Self::CandidateRevision),
            "review-report" => Some(Self::ReviewReport),
            "test-report" => Some(Self::TestReport),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::CandidateRevision => "candidate-revision",
            Self::ReviewReport => "review-report",
            Self::TestReport => "test-report",
        }
    }

    pub(crate) fn is_assurance(self) -> bool {
        matches!(self, Self::ReviewReport | Self::TestReport)
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

impl InputKey {
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

    pub(crate) fn short_hex(&self) -> String {
        self.as_hex()[..8].to_owned()
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
    #[cfg(test)]
    pub(crate) fn pin(workflow_id: Option<WorkflowId>, definition: WorkflowDefinition) -> Self {
        let version = definition.version();
        Self {
            workflow_id,
            version,
            definition,
        }
    }
}

fn assemble(
    name: String,
    default_environment: EnvironmentId,
    roles: Vec<RoleDefinition>,
    first_step: StepKey,
    mut steps: Vec<StepDefinition>,
) -> Result<WorkflowDefinition, DefinitionError> {
    let name = normalise_name(&name)?;
    if roles.len() > MAXIMUM_ROLES {
        return Err(DefinitionError::RoleCount);
    }
    if steps.is_empty() || steps.len() > MAXIMUM_STEPS {
        return Err(DefinitionError::StepCount);
    }
    for step in &mut steps {
        match &mut step.action {
            StepAction::Agent(action) => {
                action.environment = action.environment.normalised(default_environment);
            }
            StepAction::SystemCommand(action) => {
                action.environment = action.environment.normalised(default_environment);
            }
        }
    }
    reject_duplicate_roles(&roles)?;
    reject_duplicate_steps(&steps)?;
    reject_step_outputs(&steps)?;
    reject_step_inputs(&steps)?;
    reject_unsupported_outputs(&steps)?;
    reject_secondary_writes(&steps)?;
    reject_role_use(&roles, &steps)?;
    validate_serial_chain(&first_step, &steps)?;
    reject_handoff(&first_step, &steps)?;
    Ok(WorkflowDefinition {
        format_version: DEFINITION_FORMAT_VERSION,
        name,
        default_environment,
        roles,
        first_step,
        steps,
    })
}

fn from_current_file(file: DefinitionFile) -> Result<WorkflowDefinition, DefinitionError> {
    let (name, default_environment, roles, first_step, steps) = parse_file_parts(file)?;
    assemble(name, default_environment, roles, first_step, steps)
}

type FileParts = (
    String,
    EnvironmentId,
    Vec<RoleDefinition>,
    StepKey,
    Vec<StepDefinition>,
);

fn parse_file_parts(file: DefinitionFile) -> Result<FileParts, DefinitionError> {
    let name = normalise_name(&file.name)?;
    if file.roles.len() > MAXIMUM_ROLES {
        return Err(DefinitionError::RoleCount);
    }
    if file.steps.is_empty() || file.steps.len() > MAXIMUM_STEPS {
        return Err(DefinitionError::StepCount);
    }
    let default_environment =
        EnvironmentId::parse(&file.default_environment).ok_or(DefinitionError::Environment)?;
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
    Ok((name, default_environment, roles, first_step, steps))
}

fn produces_candidate_revision(step: &StepDefinition) -> bool {
    step.writes_primary_source()
        || step.command_source_effect() == Some(CommandSourceEffect::Commit)
}

fn has_secondary_write(step: &StepDefinition) -> bool {
    let StepAction::Agent(action) = &step.action else {
        return false;
    };
    action
        .authority
        .directories
        .iter()
        .any(|directory| directory.access.is_writable())
}

fn serial_order(first: &StepKey, steps: &[StepDefinition]) -> Vec<StepKey> {
    let mut order = Vec::new();
    let mut current = Some(first.clone());
    while let Some(key) = current {
        let Some(step) = steps.iter().find(|item| item.key == key) else {
            break;
        };
        order.push(key);
        current = match &step.on_success {
            SuccessTransition::Next(next) => Some(next.clone()),
            SuccessTransition::CompleteRun => None,
        };
    }
    order
}

fn parse_inputs(files: Vec<InputFile>) -> Result<Vec<RequiredInput>, DefinitionError> {
    if files.len() > MAXIMUM_INPUTS {
        return Err(DefinitionError::InputCount);
    }
    let mut inputs = Vec::with_capacity(files.len());
    for file in files {
        let key = InputKey::parse(&file.key)?;
        if inputs.iter().any(|item: &RequiredInput| item.key == key) {
            return Err(DefinitionError::DuplicateInput);
        }
        let kind = ArtefactKind::parse(&file.kind).ok_or(DefinitionError::Format)?;
        let source = match file.source {
            InputSourceFile::RunInitialCandidate => ArtefactSource::RunInitialCandidate,
            InputSourceFile::StepOutput { step, output } => ArtefactSource::StepOutput {
                step: StepKey::parse(&step)?,
                output: OutputKey::parse(&output)?,
            },
        };
        inputs.push(RequiredInput { key, kind, source });
    }
    Ok(inputs)
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
        match &step.action {
            StepAction::Agent(action) => {
                let candidate_outputs = action
                    .required_outputs
                    .iter()
                    .filter(|output| output.kind == OutputKind::CandidateRevision)
                    .count();
                let review_outputs = action
                    .required_outputs
                    .iter()
                    .filter(|output| output.kind == OutputKind::ReviewReport)
                    .count();
                match action.candidate_authority {
                    CandidateAuthority::ReadOnly if candidate_outputs != 0 => {
                        return Err(DefinitionError::CandidateOutput);
                    }
                    CandidateAuthority::Edit if candidate_outputs != 1 => {
                        return Err(DefinitionError::CandidateOutput);
                    }
                    _ => {}
                }
                if candidate_outputs == 1 && review_outputs > 1 {
                    return Err(DefinitionError::UnsupportedOutput);
                }
            }
            StepAction::SystemCommand(action) => {
                let contract = action.command.contract();
                let input_kinds: Vec<_> = step.inputs.iter().map(|input| input.kind).collect();
                let output_kinds: Vec<_> = action
                    .required_outputs
                    .iter()
                    .map(|output| output.kind)
                    .collect();
                if !kinds_match(&input_kinds, contract.required_inputs)
                    || !kinds_match(&output_kinds, contract.required_outputs)
                {
                    return Err(DefinitionError::UnsupportedOutput);
                }
            }
        }
    }
    Ok(())
}

fn reject_secondary_writes(steps: &[StepDefinition]) -> Result<(), DefinitionError> {
    for step in steps {
        if has_secondary_write(step) {
            return Err(DefinitionError::SecondaryWrite);
        }
    }
    Ok(())
}

fn reject_step_inputs(steps: &[StepDefinition]) -> Result<(), DefinitionError> {
    for step in steps {
        if step.inputs.len() > MAXIMUM_INPUTS {
            return Err(DefinitionError::InputCount);
        }
        for (index, input) in step.inputs.iter().enumerate() {
            if step
                .inputs
                .iter()
                .skip(index + 1)
                .any(|other| other.key == input.key)
            {
                return Err(DefinitionError::DuplicateInput);
            }
        }
    }
    Ok(())
}

fn reject_handoff(first: &StepKey, steps: &[StepDefinition]) -> Result<(), DefinitionError> {
    let order = serial_order(first, steps);
    let mut produced: Vec<(StepKey, OutputKey, ArtefactKind)> = Vec::new();
    let mut latest_candidate: Option<(StepKey, OutputKey)> = None;
    for key in &order {
        let step = steps
            .iter()
            .find(|item| item.key == *key)
            .ok_or(DefinitionError::UnknownStep)?;
        let candidate_inputs: Vec<_> = step
            .inputs
            .iter()
            .filter(|input| input.kind == ArtefactKind::CandidateRevision)
            .collect();
        let assurance = step.inputs.iter().any(|input| input.kind.is_assurance())
            || step.required_outputs().iter().any(|output| {
                matches!(
                    output.kind,
                    OutputKind::ReviewReport | OutputKind::TestReport
                )
            });
        if step.is_sandbox_backed() || assurance {
            if candidate_inputs.len() != 1 {
                return Err(DefinitionError::CandidateInput);
            }
            let candidate = candidate_inputs[0];
            match &candidate.source {
                ArtefactSource::RunInitialCandidate => {
                    if latest_candidate.is_some() {
                        return Err(DefinitionError::CandidateInput);
                    }
                }
                ArtefactSource::StepOutput {
                    step: source_step,
                    output,
                } => {
                    let Some((latest_step, latest_output)) = &latest_candidate else {
                        return Err(DefinitionError::CandidateInput);
                    };
                    if source_step != latest_step || output != latest_output {
                        return Err(DefinitionError::CandidateInput);
                    }
                }
            }
        } else if !candidate_inputs.is_empty() {
            return Err(DefinitionError::CandidateInput);
        }
        for input in &step.inputs {
            match &input.source {
                ArtefactSource::RunInitialCandidate => {
                    if input.kind != ArtefactKind::CandidateRevision {
                        return Err(DefinitionError::InputKind);
                    }
                }
                ArtefactSource::StepOutput {
                    step: source_step,
                    output,
                } => {
                    if source_step == &step.key {
                        return Err(DefinitionError::SelfInput);
                    }
                    if let Some(produced_kind) =
                        produced.iter().find_map(|(item_step, item_output, kind)| {
                            (*item_step == *source_step && *item_output == *output).then_some(*kind)
                        })
                    {
                        if produced_kind != input.kind {
                            return Err(DefinitionError::InputKind);
                        }
                    } else {
                        let Some(source) = steps.iter().find(|item| item.key == *source_step)
                        else {
                            return Err(DefinitionError::UnknownOutput);
                        };
                        if !order
                            .iter()
                            .take_while(|item| *item != &step.key)
                            .any(|item| item == source_step)
                        {
                            return Err(DefinitionError::ForwardInput);
                        }
                        if source.required_outputs().iter().any(|item| {
                            item.key == *output && item.kind == OutputKind::AssistantReply
                        }) {
                            return Err(DefinitionError::AssistantInput);
                        }
                        return Err(DefinitionError::UnknownOutput);
                    }
                }
            }
        }
        if assurance && candidate_inputs.is_empty() {
            return Err(DefinitionError::AssuranceInput);
        }
        let mut candidate_outputs = 0usize;
        for output in step.required_outputs() {
            if output.kind == OutputKind::CandidateRevision {
                candidate_outputs += 1;
            }
            if let Some(kind) = output.kind.as_artefact_kind() {
                produced.push((step.key.clone(), output.key.clone(), kind));
                if kind == ArtefactKind::CandidateRevision {
                    latest_candidate = Some((step.key.clone(), output.key.clone()));
                }
            }
        }
        if produces_candidate_revision(step) {
            if candidate_outputs != 1 {
                return Err(DefinitionError::CandidateOutput);
            }
        } else if candidate_outputs != 0 {
            return Err(DefinitionError::CandidateOutput);
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

pub(crate) fn initial_candidate_input() -> RequiredInput {
    RequiredInput {
        key: InputKey::parse("candidate").expect("candidate input"),
        kind: ArtefactKind::CandidateRevision,
        source: ArtefactSource::RunInitialCandidate,
    }
}

pub(crate) fn candidate_revision_output() -> RequiredOutput {
    RequiredOutput {
        key: OutputKey::parse(CANDIDATE_OUTPUT_KEY).expect("candidate output"),
        kind: OutputKind::CandidateRevision,
    }
}

#[cfg(test)]
pub(crate) fn test_environment_id() -> EnvironmentId {
    EnvironmentId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("test env")
}

#[cfg(test)]
pub(crate) fn test_named_definition(name: &str) -> WorkflowDefinition {
    let role = RoleDefinition::new(
        RoleKey::parse("agent").expect("role"),
        "Coding agent".to_owned(),
        String::new(),
        String::new(),
    )
    .expect("role");
    let authority =
        AgentAuthority::new(vec![crate::agents::ToolId::List], Vec::new()).expect("authority");
    WorkflowDefinition::from_parts(
        name.to_owned(),
        test_environment_id(),
        vec![role],
        StepKey::parse("work").expect("first"),
        vec![StepDefinition {
            key: StepKey::parse("work").expect("step"),
            name: "Work on task".to_owned(),
            inputs: vec![initial_candidate_input()],
            action: StepAction::Agent(AgentStep {
                environment: StepEnvironment::WorkflowDefault,
                role: RoleKey::parse("agent").expect("role"),
                candidate_authority: CandidateAuthority::Edit,
                authority,
                required_outputs: vec![
                    RequiredOutput {
                        key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                        kind: OutputKind::AssistantReply,
                    },
                    candidate_revision_output(),
                ],
            }),
            on_success: SuccessTransition::CompleteRun,
        }],
    )
    .expect("definition")
}

#[cfg(test)]
mod tests;
