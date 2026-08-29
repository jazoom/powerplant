use std::path::PathBuf;

use super::record::{
    AccessMode, AgentError, AgentRecord, DirectoryGrant, GUEST_PROJECT, canonical_directory,
    guest_path_for,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyGrant {
    pub(crate) alias: String,
    pub(crate) guest_path: String,
    pub(crate) host_path: PathBuf,
    pub(crate) access: AccessMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectoryPolicy {
    grants: Vec<PolicyGrant>,
    primary_alias: String,
}

impl DirectoryPolicy {
    pub(crate) fn from_record(record: &AgentRecord) -> Self {
        let grants = record
            .directories
            .iter()
            .map(|grant| PolicyGrant {
                alias: grant.alias.clone(),
                guest_path: guest_path_for(&grant.alias, &record.primary_directory),
                host_path: grant.host_path.clone(),
                access: grant.access,
            })
            .collect();
        Self {
            grants,
            primary_alias: record.primary_directory.clone(),
        }
    }

    pub(crate) fn from_grants(grants: Vec<PolicyGrant>, primary_alias: String) -> Self {
        Self {
            grants,
            primary_alias,
        }
    }

    pub(crate) fn grants(&self) -> &[PolicyGrant] {
        &self.grants
    }

    pub(crate) fn primary_alias(&self) -> &str {
        &self.primary_alias
    }

    pub(crate) fn primary_guest(&self) -> &str {
        self.grants
            .iter()
            .find(|grant| grant.alias == self.primary_alias)
            .map(|grant| grant.guest_path.as_str())
            .unwrap_or(GUEST_PROJECT)
    }

    pub(crate) fn primary_access(&self) -> AccessMode {
        self.grants
            .iter()
            .find(|grant| grant.alias == self.primary_alias)
            .map(|grant| grant.access)
            .unwrap_or(AccessMode::ReadOnly)
    }

    pub(crate) fn resolve(&self, raw: &str) -> Result<(String, AccessMode), &'static str> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok((self.primary_guest().to_owned(), self.primary_access()));
        }
        if raw.chars().any(char::is_control) {
            return Err("That path is not valid.");
        }
        let joined = if raw.starts_with('/') {
            raw.to_owned()
        } else {
            format!("{}/{raw}", self.primary_guest())
        };
        let normalised = normalise_absolute(&joined)?;
        self.grant_for(&normalised)
            .map(|grant| (normalised, grant.access))
            .ok_or("Stay inside a granted directory.")
    }

    pub(crate) fn guest_roots(&self) -> Vec<String> {
        self.grants
            .iter()
            .map(|grant| grant.guest_path.clone())
            .collect()
    }

    pub(crate) fn writable_roots(&self) -> Vec<String> {
        self.grants
            .iter()
            .filter(|grant| grant.access.is_writable())
            .map(|grant| grant.guest_path.clone())
            .collect()
    }

    pub(crate) fn confirm_hosts(&self) -> Result<(), AgentError> {
        for grant in &self.grants {
            let resolved = canonical_directory(&grant.host_path)?;
            if resolved != grant.host_path {
                return Err(AgentError::Path);
            }
        }
        Ok(())
    }

    fn grant_for(&self, path: &str) -> Option<&PolicyGrant> {
        self.grants.iter().find(|grant| {
            path == grant.guest_path || path.starts_with(&format!("{}/", grant.guest_path))
        })
    }
}

pub(crate) fn grants_changed(
    current: &[DirectoryGrant],
    next: &[DirectoryGrant],
    primary: &str,
    next_primary: &str,
) -> bool {
    current != next || primary != next_primary
}

fn normalise_absolute(path: &str) -> Result<String, &'static str> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if parts.is_empty() {
                return Err("Stay inside a granted directory.");
            }
            parts.pop();
            continue;
        }
        parts.push(part);
    }
    Ok(format!("/{}", parts.join("/")))
}

#[cfg(test)]
mod tests;
