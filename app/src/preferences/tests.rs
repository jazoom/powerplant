use std::fs;

use super::{Preferences, Theme};

#[test]
fn a_saved_theme_survives_a_new_store_instance() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("preferences.json");

    let writer = Preferences::open(path.clone());
    writer
        .set_theme(Theme::SpringfieldDark)
        .expect("save theme");

    let reader = Preferences::open(path);
    assert_eq!(reader.theme(), Theme::SpringfieldDark);
}

#[test]
fn missing_or_invalid_files_default_to_light() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("preferences.json");
    assert_eq!(
        Preferences::open(path.clone()).theme(),
        Theme::SpringfieldLight
    );

    for bytes in [
        b"not json".as_slice(),
        br#"{"version":2,"theme":"springfield-dark"}"#,
        br#"{"version":1,"theme":"unknown"}"#,
    ] {
        fs::write(&path, bytes).expect("write invalid preferences");
        assert_eq!(
            Preferences::open(path.clone()).theme(),
            Theme::SpringfieldLight
        );
    }
}

#[test]
fn only_known_themes_parse() {
    assert_eq!(
        Theme::parse("springfield-light"),
        Some(Theme::SpringfieldLight)
    );
    assert_eq!(
        Theme::parse("springfield-dark"),
        Some(Theme::SpringfieldDark)
    );
    assert_eq!(Theme::parse("Springfield dark"), None);
    assert_eq!(Theme::parse("unknown"), None);
}
