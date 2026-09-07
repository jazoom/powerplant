pub(crate) mod authority;
mod settings;

pub(crate) use authority::ProjectFreeAuthority;
pub(crate) use settings::ExecutionSettings;

pub(crate) const GUEST_WORKSPACE: &str = "/workspace";
