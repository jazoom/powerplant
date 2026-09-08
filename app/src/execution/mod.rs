mod approval;
pub(crate) mod authority;
mod consent;
mod folder_picker;
mod host;
mod overrides;
mod settings;
pub(crate) use overrides::{SettingsOverrides, SettingsOverridesFile};

pub(crate) use approval::{
    ApprovalError, HostApprovalStore, HostCommandDecision, HostCommandRequest,
};
pub(crate) use authority::ProjectFreeAuthority;
pub(crate) use consent::{AccessConsentStore, draft_nonce};
pub(crate) use folder_picker::{FolderPick, FolderPicker};
pub(crate) use host::{COMMAND_TIMEOUT, HostIdentity, command_directory, run_shell};
pub(crate) use settings::{
    CanonicalDirectoryIdentity, DirectoryAccess, DirectoryGrant, DirectoryGrantError,
    DirectoryGrantId, ExecutionSettings, ExecutionSettingsFile, ToolLocation, valid_alias,
    validate_directories,
};

pub(crate) const GUEST_WORKSPACE: &str = "/workspace";
