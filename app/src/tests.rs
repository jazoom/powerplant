pub(crate) use crate::environments::tests::{FailureCategory, sample_snapshot};
pub(crate) use crate::providers::tests::ScriptedBackend;
pub(crate) use crate::state::tests::test_state;
pub(crate) use crate::workflows::tests::{
    test_agent_capabilities, test_command_capabilities, test_environment_id, test_environment_set,
    test_named_definition,
};

pub(crate) fn desk_path(
    project_id: &crate::projects::ProjectId,
    agent_id: &crate::agents::AgentId,
) -> String {
    format!(
        "/projects/{}/agents/{}",
        project_id.as_hex(),
        agent_id.as_hex()
    )
}
