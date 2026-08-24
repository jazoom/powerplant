use super::StartupConfig;
use std::collections::HashMap;

fn development_values() -> HashMap<String, String> {
    [("CIRCUS_ENVIRONMENT", "development")]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[test]
fn parses_development_defaults() {
    let config = StartupConfig::from_values(development_values()).unwrap();
    assert_eq!(config.runtime.public_origin(), "http://localhost:4000");
    assert_eq!(config.bind_address, "localhost:4000");
}

#[test]
fn production_requires_origin() {
    let mut values = development_values();
    values.insert("CIRCUS_ENVIRONMENT".into(), "production".into());
    assert_eq!(
        StartupConfig::from_values(values).err().unwrap(),
        "CIRCUS_PUBLIC_ORIGIN must be set"
    );
}

#[test]
fn rejects_empty_environment() {
    let mut values = development_values();
    values.insert("CIRCUS_ENVIRONMENT".into(), String::new());
    assert_eq!(
        StartupConfig::from_values(values).err().unwrap(),
        "CIRCUS_ENVIRONMENT must not be empty"
    );
}

#[test]
fn rejects_relative_static_dir() {
    let mut values = development_values();
    values.insert("CIRCUS_STATIC_DIR".into(), "static".into());
    assert_eq!(
        StartupConfig::from_values(values).err().unwrap(),
        "CIRCUS_STATIC_DIR must be absolute"
    );
}

#[test]
fn rejects_non_canonical_origin() {
    let mut values = development_values();
    values.insert(
        "CIRCUS_PUBLIC_ORIGIN".into(),
        "http://localhost:4000/".into(),
    );
    assert_eq!(
        StartupConfig::from_values(values).err().unwrap(),
        "CIRCUS_PUBLIC_ORIGIN must be canonical"
    );
}
