mod id;
mod record;
mod store;

pub(crate) use id::ProjectId;
pub(crate) use record::{ProjectError, ProjectRecord, submitted_host_path, submitted_name};
pub(crate) use store::ProjectStore;

use crate::agents::{AgentRecord, DirectoryGrant};

pub(crate) fn exact_grant<'a>(
    agent: &'a AgentRecord,
    project: &ProjectRecord,
) -> Option<&'a DirectoryGrant> {
    agent
        .directories
        .iter()
        .find(|grant| grant.host_path == project.host_path)
}
