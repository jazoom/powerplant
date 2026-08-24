use std::{collections::HashMap, fs, path::Path};

use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AssetPaths {
    pub(crate) css_path: String,
    pub(crate) js_path: String,
}

impl AssetPaths {
    pub(crate) fn load(static_dir: &Path) -> Result<Self, String> {
        let manifest_path = static_dir.join(".vite/manifest.json");
        let manifest = fs::read_to_string(&manifest_path).map_err(|error| {
            format!(
                "could not read app static manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        Self::parse(&manifest).map_err(|error| {
            format!(
                "invalid app static manifest {}: {error}",
                manifest_path.display()
            )
        })
    }

    fn parse(manifest: &str) -> Result<Self, String> {
        let entries: HashMap<String, Value> = serde_json::from_str(manifest)
            .map_err(|error| format!("could not parse JSON: {error}"))?;
        let entry = entries
            .get("assets/main.ts")
            .ok_or_else(|| "manifest is missing assets/main.ts".to_owned())?;
        let file = entry
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| "assets/main.ts has no JavaScript file".to_owned())?;
        let css = entry
            .get("css")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str)
            .ok_or_else(|| "assets/main.ts has no CSS file".to_owned())?;
        Ok(Self {
            css_path: format!("/static/{css}"),
            js_path: format!("/static/{file}"),
        })
    }
}

#[cfg(test)]
mod tests;
