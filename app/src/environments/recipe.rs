use serde::Serialize;
use sha2::{Digest, Sha256};

use microsandbox::sandbox::{IntoImage, RootfsSource};

use crate::hex;

pub(crate) const MAXIMUM_NAME_BYTES: usize = 80;
pub(crate) const MAXIMUM_IMAGE_BYTES: usize = 512;
pub(crate) const MAXIMUM_SCRIPT_BYTES: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnvironmentDraft {
    pub(crate) name: String,
    pub(crate) oci_image: String,
    pub(crate) setup_script: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnvironmentRecipe {
    pub(crate) oci_image: OciImageReference,
    pub(crate) setup_script: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OciImageReference(String);

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct EnvironmentRecipeVersion([u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecipeError {
    Name,
    Image,
    Script,
    LocalPath,
    DiskImage,
    Archive,
}

impl RecipeError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Name => "Enter a name of at most 80 bytes.",
            Self::Image => "Enter an OCI base image of at most 512 ASCII bytes.",
            Self::Script => "That setup script is too long or contains disallowed characters.",
            Self::LocalPath => "Use an OCI image reference, not a local path.",
            Self::DiskImage => "Use an OCI image reference, not a disk image.",
            Self::Archive => "Use an OCI image reference, not an image archive.",
        }
    }
}

impl EnvironmentRecipe {
    pub(crate) fn from_draft(draft: &EnvironmentDraft) -> Result<(String, Self), RecipeError> {
        let name = normalise_name(&draft.name)?;
        let oci_image = OciImageReference::parse(&draft.oci_image)?;
        let setup_script = normalise_script(&draft.setup_script)?;
        Ok((
            name,
            Self {
                oci_image,
                setup_script,
            },
        ))
    }

    pub(crate) fn version(&self) -> EnvironmentRecipeVersion {
        EnvironmentRecipeVersion::of(self)
    }
}

impl OciImageReference {
    pub(crate) fn parse(raw: &str) -> Result<Self, RecipeError> {
        let value = raw.trim();
        if value.is_empty()
            || value.len() > MAXIMUM_IMAGE_BYTES
            || !value.is_ascii()
            || value.chars().any(char::is_control)
        {
            return Err(RecipeError::Image);
        }
        let lowered = value.to_ascii_lowercase();
        if lowered.starts_with("docker-archive:") || lowered.starts_with("oci-archive:") {
            return Err(RecipeError::Archive);
        }
        match value.into_rootfs_source() {
            Ok(RootfsSource::Oci(_)) => {}
            Ok(RootfsSource::DiskImage { .. }) => return Err(RecipeError::DiskImage),
            Ok(_) => return Err(RecipeError::LocalPath),
            Err(_) => return Err(RecipeError::Image),
        }
        if has_disk_image_extension(value) {
            return Err(RecipeError::DiskImage);
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl EnvironmentRecipeVersion {
    fn of(recipe: &EnvironmentRecipe) -> Self {
        let file = RecipeCanonical {
            oci_image: recipe.oci_image.as_str(),
            setup_script: &recipe.setup_script,
        };
        let bytes = serde_json::to_vec(&file).expect("canonical recipe json");
        Self(Sha256::digest(bytes).into())
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        hex::decode(value).map(Self)
    }

    pub(crate) fn as_hex(&self) -> String {
        hex::encode(&self.0)
    }

    pub(crate) fn as_digest(&self) -> String {
        format!("sha256:{}", self.as_hex())
    }

    pub(crate) fn short_hex(&self) -> String {
        self.as_hex()[..8].to_owned()
    }
}

impl std::fmt::Debug for EnvironmentRecipeVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EnvironmentRecipeVersion(")?;
        formatter.write_str(&self.as_hex())?;
        formatter.write_str(")")
    }
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct RecipeCanonical<'a> {
    oci_image: &'a str,
    setup_script: &'a str,
}

pub(crate) fn normalise_name(raw: &str) -> Result<String, RecipeError> {
    let name = raw.trim();
    if name.is_empty() || name.len() > MAXIMUM_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(RecipeError::Name);
    }
    Ok(name.to_owned())
}

fn normalise_script(raw: &str) -> Result<String, RecipeError> {
    let script = raw.replace("\r\n", "\n").replace('\r', "\n");
    if script.len() > MAXIMUM_SCRIPT_BYTES {
        return Err(RecipeError::Script);
    }
    if script
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(RecipeError::Script);
    }
    Ok(script)
}

fn has_disk_image_extension(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    lowered.ends_with(".qcow2")
        || lowered.ends_with(".raw")
        || lowered.ends_with(".vmdk")
        || lowered.ends_with(".img")
}

#[cfg(test)]
mod tests;
