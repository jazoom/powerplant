use std::path::{Path, PathBuf};

use rand::rand_core::TryRng;
use rand::rngs::SysRng;
use serde::{Deserialize, Serialize};

use crate::{
    agents::{AccessMode, NetworkAccess, ToolId},
    providers::ModelSelection,
};

pub(crate) const MAXIMUM_INSTRUCTION_BYTES: usize = crate::agents::MAXIMUM_INSTRUCTION_BYTES;
pub(crate) const MAXIMUM_DIRECTORY_GRANTS: usize = 8;
const MAXIMUM_FORM_GRANT_BYTES: usize = 8 * 1024;
const MAXIMUM_ALIAS_BYTES: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionSettings {
    pub(crate) model: ModelSelection,
    pub(crate) instructions: String,
    pub(crate) tools: Vec<ToolId>,
    pub(crate) network: NetworkAccess,
    pub(crate) directories: Vec<DirectoryGrant>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectoryGrant {
    pub(crate) id: DirectoryGrantId,
    pub(crate) host_path: PathBuf,
    pub(crate) identity: CanonicalDirectoryIdentity,
    pub(crate) alias: String,
    pub(crate) access: AccessMode,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DirectoryGrantId([u8; 16]);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CanonicalDirectoryIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DirectoryGrantError {
    Random,
    Path,
    Unavailable,
    Duplicate,
    Overlap,
    Full,
    Invalid,
    Sensitive,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DirectoryGrantForm {
    id: String,
    host_path: PathBuf,
    identity: CanonicalDirectoryIdentity,
    alias: String,
    access: String,
}

impl ExecutionSettings {
    pub(crate) fn new(
        model: ModelSelection,
        instructions: String,
        tools: Vec<ToolId>,
    ) -> Option<Self> {
        if instructions.len() > MAXIMUM_INSTRUCTION_BYTES
            || instructions
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
            || tools.len() > ToolId::ALL.len()
            || tools
                .iter()
                .enumerate()
                .any(|(index, tool)| tools[..index].contains(tool))
        {
            return None;
        }
        Some(Self {
            model,
            instructions,
            tools,
            network: NetworkAccess::None,
            directories: Vec::new(),
        })
    }

    pub(crate) fn with_network(mut self, network: NetworkAccess) -> Option<Self> {
        self.network = network.validate().ok()?;
        Some(self)
    }

    pub(crate) fn with_directories(mut self, directories: Vec<DirectoryGrant>) -> Option<Self> {
        validate_directories(&directories).ok()?;
        self.directories = directories;
        Some(self)
    }
}

impl DirectoryGrant {
    pub(crate) fn from_selected(
        selected: &Path,
        existing: &[Self],
    ) -> Result<Self, DirectoryGrantError> {
        if existing.len() >= MAXIMUM_DIRECTORY_GRANTS {
            return Err(DirectoryGrantError::Full);
        }
        let host_path = canonical_directory(selected)?;
        let identity = directory_identity(&host_path)?;
        if existing
            .iter()
            .any(|grant| grant.host_path == host_path || grant.identity == identity)
        {
            return Err(DirectoryGrantError::Duplicate);
        }
        if existing
            .iter()
            .any(|grant| paths_overlap(&grant.host_path, &host_path))
        {
            return Err(DirectoryGrantError::Overlap);
        }
        let alias = available_alias(&host_path, existing);
        Ok(Self {
            id: DirectoryGrantId::generate()?,
            host_path,
            identity,
            alias,
            access: AccessMode::ReadOnly,
        })
    }

    pub(crate) fn revalidate(&self) -> Result<(), DirectoryGrantError> {
        let canonical = canonical_directory(&self.host_path)?;
        if canonical != self.host_path || directory_identity(&canonical)? != self.identity {
            return Err(DirectoryGrantError::Unavailable);
        }
        Ok(())
    }

    pub(crate) fn is_available(&self) -> bool {
        self.revalidate().is_ok()
    }

    pub(crate) fn guest_path(&self) -> String {
        format!("/access/{}", self.alias)
    }

    pub(crate) fn form_value(&self) -> String {
        serde_json::to_string(&DirectoryGrantForm {
            id: self.id.as_hex(),
            host_path: self.host_path.clone(),
            identity: self.identity,
            alias: self.alias.clone(),
            access: self.access.as_str().to_owned(),
        })
        .expect("directory grant form")
    }

    pub(crate) fn parse_form(value: &str) -> Option<Self> {
        if value.len() > MAXIMUM_FORM_GRANT_BYTES {
            return None;
        }
        let form: DirectoryGrantForm = serde_json::from_str(value).ok()?;
        let grant = Self {
            id: DirectoryGrantId::parse(&form.id)?,
            host_path: form.host_path,
            identity: form.identity,
            alias: form.alias,
            access: AccessMode::parse(&form.access)?,
        };
        validate_directories(std::slice::from_ref(&grant)).ok()?;
        Some(grant)
    }
}

impl DirectoryGrantId {
    pub(crate) fn generate() -> Result<Self, DirectoryGrantError> {
        let mut bytes = [0_u8; 16];
        SysRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| DirectoryGrantError::Random)?;
        Ok(Self(bytes))
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        crate::hex::decode(value).map(Self)
    }

    pub(crate) fn as_hex(self) -> String {
        crate::hex::encode(&self.0)
    }
}

impl DirectoryGrantError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Random => "Power Plant could not create a directory grant. Try again.",
            Self::Path => "Choose an absolute directory path.",
            Self::Unavailable => "That directory is unavailable or changed at the saved path.",
            Self::Duplicate => "That directory already has access.",
            Self::Overlap => "Directory grants cannot overlap.",
            Self::Full => "This conversation has the maximum of eight directories.",
            Self::Invalid => "That directory grant is not valid.",
            Self::Sensitive => {
                "Access to this sensitive directory needs explicit approval. This approval is not available yet."
            }
        }
    }
}

pub(crate) fn validate_directories(
    directories: &[DirectoryGrant],
) -> Result<(), DirectoryGrantError> {
    if directories.len() > MAXIMUM_DIRECTORY_GRANTS {
        return Err(DirectoryGrantError::Full);
    }
    for (index, grant) in directories.iter().enumerate() {
        if !valid_stored_path(&grant.host_path)
            || !valid_alias(&grant.alias)
            || grant.access != AccessMode::ReadOnly
            || directories[..index].iter().any(|previous| {
                previous.id == grant.id
                    || previous.alias == grant.alias
                    || previous.host_path == grant.host_path
                    || previous.identity == grant.identity
            })
        {
            return Err(DirectoryGrantError::Invalid);
        }
        if directories[..index]
            .iter()
            .any(|previous| paths_overlap(&previous.host_path, &grant.host_path))
        {
            return Err(DirectoryGrantError::Overlap);
        }
    }
    Ok(())
}

fn canonical_directory(path: &Path) -> Result<PathBuf, DirectoryGrantError> {
    if !valid_stored_path(path) {
        return Err(DirectoryGrantError::Path);
    }
    let metadata = std::fs::metadata(path).map_err(|_| DirectoryGrantError::Unavailable)?;
    if !metadata.is_dir() {
        return Err(DirectoryGrantError::Unavailable);
    }
    std::fs::canonicalize(path).map_err(|_| DirectoryGrantError::Unavailable)
}

fn valid_stored_path(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    path.is_absolute()
        && raw.len() <= crate::agents::MAXIMUM_PATH_BYTES
        && !raw.chars().any(char::is_control)
}

#[cfg(unix)]
fn directory_identity(path: &Path) -> Result<CanonicalDirectoryIdentity, DirectoryGrantError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).map_err(|_| DirectoryGrantError::Unavailable)?;
    Ok(CanonicalDirectoryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn directory_identity(_path: &Path) -> Result<CanonicalDirectoryIdentity, DirectoryGrantError> {
    Err(DirectoryGrantError::Unavailable)
}

fn available_alias(path: &Path, existing: &[DirectoryGrant]) -> String {
    let raw = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("directory");
    let mut base = String::new();
    let mut separator = false;
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !base.is_empty() {
                base.push('-');
            }
            separator = false;
            base.push(character.to_ascii_lowercase());
        } else {
            separator = true;
        }
    }
    if base.is_empty() {
        base.push_str("directory");
    }
    if !base.as_bytes()[0].is_ascii_alphabetic() {
        base.insert_str(0, "d-");
    }
    base.truncate(MAXIMUM_ALIAS_BYTES);
    if valid_alias(&base) && existing.iter().all(|grant| grant.alias != base) {
        return base;
    }
    for number in 2..=MAXIMUM_DIRECTORY_GRANTS + 1 {
        let suffix = format!("-{number}");
        let mut candidate = base.clone();
        candidate.truncate(MAXIMUM_ALIAS_BYTES - suffix.len());
        candidate.push_str(&suffix);
        if existing.iter().all(|grant| grant.alias != candidate) {
            return candidate;
        }
    }
    unreachable!("the grant bound leaves an alias available")
}

fn valid_alias(alias: &str) -> bool {
    // Workflow authority reserves this alias for its legacy primary source.
    alias != "project"
        && !alias.is_empty()
        && alias.len() <= MAXIMUM_ALIAS_BYTES
        && alias.as_bytes()[0].is_ascii_alphabetic()
        && alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

#[cfg(test)]
mod tests;
