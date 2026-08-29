use crate::agents::{AgentRecord, DirectoryPolicy};

use super::definition::{
    ASSISTANT_REPLY, AgentAuthority, AgentStep, GuestDirectoryAccess, OutputKey, OutputKind,
    RequiredOutput, RoleDefinition, RoleKey, StepAction, StepDefinition, StepKey,
    SuccessTransition, WorkflowDefinition,
};

const ROLE_KEY: &str = "agent";
const STEP_KEY: &str = "reply";
const STEP_NAME: &str = "Reply";

pub(crate) fn compatibility_definition(
    record: &AgentRecord,
) -> Result<WorkflowDefinition, super::definition::DefinitionError> {
    let role = RoleDefinition::new(
        RoleKey::parse(ROLE_KEY)?,
        record.name.clone(),
        String::new(),
        record.instructions.clone(),
    )?;
    let directories = record
        .directories
        .iter()
        .map(|grant| GuestDirectoryAccess {
            alias: grant.alias.clone(),
            access: grant.access,
        })
        .collect();
    let authority = AgentAuthority::new(record.tools.clone(), directories)?;
    let step = StepDefinition {
        key: StepKey::parse(STEP_KEY)?,
        name: STEP_NAME.to_owned(),
        action: StepAction::Agent(AgentStep {
            role: RoleKey::parse(ROLE_KEY)?,
            authority,
            required_outputs: vec![RequiredOutput {
                key: OutputKey::parse(ASSISTANT_REPLY)?,
                kind: OutputKind::AssistantReply,
            }],
        }),
        on_success: SuccessTransition::CompleteRun,
    };
    WorkflowDefinition::from_parts(
        record.name.clone(),
        vec![role],
        StepKey::parse(STEP_KEY)?,
        vec![step],
    )
}

pub(crate) fn compose_preamble(
    role: &RoleDefinition,
    authority: &AgentAuthority,
    policy: &DirectoryPolicy,
) -> String {
    crate::agents::compose_role(
        &role.name,
        &role.expertise,
        &role.prompt_defaults,
        &authority.tools,
        policy,
    )
}

#[cfg(test)]
mod tests;
