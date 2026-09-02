use super::*;

impl super::RuntimeConfig {
    pub(crate) fn development() -> Self {
        Self {
            environment: RuntimeEnvironment::Development,
            public_origin: "http://localhost:4000".to_owned(),
        }
    }
}

use super::StartupConfig;
use std::collections::HashMap;

fn development_values() -> HashMap<String, String> {
    [
        ("POWERPLANT_ENVIRONMENT", "development"),
        ("HOME", "/home/powerplant"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

#[test]
fn parses_development_defaults() {
    let config = StartupConfig::from_values(development_values()).unwrap();
    assert_eq!(config.runtime.public_origin(), "http://localhost:4000");
    assert_eq!(config.bind_address, "localhost:4000");
    assert_eq!(
        config.data_dir,
        std::path::PathBuf::from("/home/powerplant/.local/share/powerplant")
    );
}

#[test]
fn prefers_an_absolute_data_dir() {
    let mut values = development_values();
    values.insert("POWERPLANT_DATA_DIR".into(), "/var/lib/powerplant".into());
    let config = StartupConfig::from_values(values).unwrap();
    assert_eq!(
        config.data_dir,
        std::path::PathBuf::from("/var/lib/powerplant")
    );
}

#[test]
fn rejects_a_relative_data_dir() {
    let mut values = development_values();
    values.insert("POWERPLANT_DATA_DIR".into(), "data".into());
    assert_eq!(
        StartupConfig::from_values(values).err().unwrap(),
        "POWERPLANT_DATA_DIR must be absolute"
    );
}

#[test]
fn production_requires_origin() {
    let mut values = development_values();
    values.insert("POWERPLANT_ENVIRONMENT".into(), "production".into());
    assert_eq!(
        StartupConfig::from_values(values).err().unwrap(),
        "POWERPLANT_PUBLIC_ORIGIN must be set"
    );
}

#[test]
fn rejects_empty_environment() {
    let mut values = development_values();
    values.insert("POWERPLANT_ENVIRONMENT".into(), String::new());
    assert_eq!(
        StartupConfig::from_values(values).err().unwrap(),
        "POWERPLANT_ENVIRONMENT must not be empty"
    );
}

#[test]
fn rejects_relative_static_dir() {
    let mut values = development_values();
    values.insert("POWERPLANT_STATIC_DIR".into(), "static".into());
    assert_eq!(
        StartupConfig::from_values(values).err().unwrap(),
        "POWERPLANT_STATIC_DIR must be absolute"
    );
}

#[test]
fn rejects_non_canonical_origin() {
    let mut values = development_values();
    values.insert(
        "POWERPLANT_PUBLIC_ORIGIN".into(),
        "http://localhost:4000/".into(),
    );
    assert_eq!(
        StartupConfig::from_values(values).err().unwrap(),
        "POWERPLANT_PUBLIC_ORIGIN must be canonical"
    );
}
