use crate::agents::{DirectoryPolicy, NetworkAccess, ToolId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectFreeAuthority {
    pub(crate) revision: u32,
    pub(crate) tools: Vec<ToolId>,
    pub(crate) network: NetworkAccess,
    pub(crate) policy: DirectoryPolicy,
}

impl ProjectFreeAuthority {
    pub(crate) fn from_settings(revision: u32, settings: &super::ExecutionSettings) -> Self {
        Self {
            revision,
            tools: settings.tools.clone(),
            network: settings.network.clone(),
            policy: DirectoryPolicy::private_workspace(),
        }
    }
}

#[cfg(test)]
mod tests;
