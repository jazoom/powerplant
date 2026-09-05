mod eligibility;
mod folder_picker;
mod id;
mod record;
mod store;

pub(crate) use eligibility::{
    EligibleGrant, eligibility, eligible_agents, eligible_projects, exact_grant,
};
pub(crate) use folder_picker::{FolderPick, ProjectFolderPicker};
pub(crate) use id::ProjectId;
pub(crate) use record::{
    MAXIMUM_PROJECTS, ProjectError, ProjectRecord, submitted_host_path, submitted_name,
};
pub(crate) use store::ProjectStore;

use crate::agents::AgentId;

pub(crate) fn desk_path(project_id: &ProjectId, agent_id: &AgentId) -> String {
    format!(
        "/projects/{}/agents/{}",
        project_id.as_hex(),
        agent_id.as_hex()
    )
}
