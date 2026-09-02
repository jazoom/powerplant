mod eligibility;
mod id;
mod record;
mod store;

pub(crate) use eligibility::{
    EligibleGrant, eligibility, eligible_agents, eligible_projects, exact_grant,
};
pub(crate) use id::ProjectId;
pub(crate) use record::{
    MAXIMUM_PROJECTS, ProjectError, ProjectRecord, submitted_host_path, submitted_name,
};
pub(crate) use store::ProjectStore;

use crate::agents::{AgentId, AgentRecord};

pub(crate) fn desk_path(project_id: &ProjectId, agent_id: &AgentId) -> String {
    format!(
        "/projects/{}/agents/{}",
        project_id.as_hex(),
        agent_id.as_hex()
    )
}

pub(crate) fn unique_desk_path(agent: &AgentRecord, projects: &[ProjectRecord]) -> Option<String> {
    let mut matched = eligible_projects(agent, projects).into_iter();
    let first = matched.next()?;
    if matched.next().is_some() {
        return None;
    }
    Some(desk_path(&first.id, &agent.id))
}
