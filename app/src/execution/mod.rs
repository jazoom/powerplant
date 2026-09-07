pub(crate) mod authority;
mod folder_picker;
mod settings;

pub(crate) use authority::ProjectFreeAuthority;
pub(crate) use folder_picker::{FolderPick, FolderPicker};
pub(crate) use settings::{
    CanonicalDirectoryIdentity, DirectoryGrant, DirectoryGrantError, DirectoryGrantId,
    ExecutionSettings, validate_directories,
};

pub(crate) const GUEST_WORKSPACE: &str = "/workspace";
