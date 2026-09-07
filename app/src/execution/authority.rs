use std::path::Path;

use crate::agents::{AccessMode, DirectoryPolicy, NetworkAccess, PolicyGrant, ToolId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectFreeAuthority {
    pub(crate) revision: u32,
    pub(crate) tools: Vec<ToolId>,
    pub(crate) network: NetworkAccess,
    pub(crate) policy: DirectoryPolicy,
}

impl ProjectFreeAuthority {
    pub(crate) fn from_settings(
        revision: u32,
        settings: &super::ExecutionSettings,
    ) -> Result<Self, super::DirectoryGrantError> {
        super::settings::validate_directories(&settings.directories)?;
        for grant in &settings.directories {
            grant.revalidate()?;
        }
        let primary_alias = settings
            .directories
            .first()
            .map(|grant| grant.alias.clone())
            .unwrap_or_default();
        let grants = settings
            .directories
            .iter()
            .map(|grant| PolicyGrant {
                alias: grant.alias.clone(),
                guest_path: grant.guest_path(),
                host_path: grant.host_path.clone(),
                access: AccessMode::ReadOnly,
            })
            .collect();
        Ok(Self {
            revision,
            tools: settings.tools.clone(),
            network: settings.network.clone(),
            policy: DirectoryPolicy::from_grants_with_workspace(grants, primary_alias),
        })
    }
}

pub(crate) fn sensitive_directory(path: &Path, data_root: &Path) -> bool {
    if paths_overlap(path, data_root) {
        return true;
    }
    std::env::var_os("HOME")
        .and_then(|home| std::fs::canonicalize(home).ok())
        .is_some_and(|home| path == home || home.starts_with(path))
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

#[cfg(test)]
mod tests;
