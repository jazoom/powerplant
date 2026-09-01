use crate::agents::{AccessMode, AgentRecord, DirectoryGrant};

use super::record::ProjectRecord;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EligibleGrant {
    pub(crate) alias: String,
    pub(crate) access: AccessMode,
}

// Eligibility is exact stored-path equality. A parent or prefix grant is not authority.
pub(crate) fn exact_grant<'a>(
    agent: &'a AgentRecord,
    project: &ProjectRecord,
) -> Option<&'a DirectoryGrant> {
    agent
        .directories
        .iter()
        .find(|grant| grant.host_path == project.host_path)
}

pub(crate) fn eligibility(agent: &AgentRecord, project: &ProjectRecord) -> Option<EligibleGrant> {
    exact_grant(agent, project).map(|grant| EligibleGrant {
        alias: grant.alias.clone(),
        access: grant.access,
    })
}

pub(crate) fn eligible_agents(agents: &[AgentRecord], project: &ProjectRecord) -> Vec<AgentRecord> {
    agents
        .iter()
        .filter(|agent| exact_grant(agent, project).is_some())
        .cloned()
        .collect()
}
