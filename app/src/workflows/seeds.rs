use crate::agents::{AccessMode, ToolId};

use super::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, GuestDirectoryAccess, OutputKey, OutputKind,
    RequiredOutput, RoleDefinition, RoleKey, StepAction, StepDefinition, StepKey,
    SuccessTransition, WorkflowDefinition,
};

pub(crate) const ONE_AGENT_V1: &str = "one-agent-v1";

const SEED_KEY_BYTES: usize = 32;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct SeedKey(String);

#[derive(Clone, Debug)]
pub(crate) struct WorkflowSeed {
    pub(crate) key: SeedKey,
    pub(crate) definition: WorkflowDefinition,
}

impl SeedKey {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let key = value.trim();
        if key.is_empty() || key.len() > SEED_KEY_BYTES {
            return None;
        }
        let mut characters = key.chars();
        let first = characters.next()?;
        if !first.is_ascii_alphabetic() {
            return None;
        }
        if !characters.all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        }) {
            return None;
        }
        Some(Self(key.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) fn production_seeds() -> Vec<WorkflowSeed> {
    vec![WorkflowSeed {
        key: SeedKey::parse(ONE_AGENT_V1).expect("one-agent seed key"),
        definition: one_agent_definition(),
    }]
}

pub(crate) fn one_agent_definition() -> WorkflowDefinition {
    let role = RoleDefinition::new(
        RoleKey::parse("coding-agent").expect("role"),
        "Coding agent".to_owned(),
        String::new(),
        String::new(),
    )
    .expect("role");
    let authority = AgentAuthority::new(
        ToolId::ALL.to_vec(),
        vec![GuestDirectoryAccess {
            alias: "project".to_owned(),
            access: AccessMode::ReadWrite,
        }],
    )
    .expect("authority");
    let step = StepDefinition {
        key: StepKey::parse("work-on-task").expect("step"),
        name: "Work on task".to_owned(),
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse("coding-agent").expect("role"),
            authority,
            required_outputs: vec![RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY).expect("output"),
                kind: OutputKind::AssistantReply,
            }],
        }),
        on_success: SuccessTransition::CompleteRun,
    };
    WorkflowDefinition::from_parts(
        "One agent".to_owned(),
        vec![role],
        StepKey::parse("work-on-task").expect("first"),
        vec![step],
    )
    .expect("one agent definition")
}

#[cfg(test)]
mod tests;
