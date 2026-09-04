use std::fs;

use super::{Preferences, Theme};

#[test]
fn a_saved_theme_survives_a_new_store_instance() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("preferences.json");

    let writer = Preferences::open(path.clone());
    writer.set_theme(Theme::Sector7G).expect("save theme");

    let reader = Preferences::open(path);
    assert_eq!(reader.theme(), Theme::Sector7G);
}

#[test]
fn missing_or_invalid_files_default_to_light() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("preferences.json");
    assert_eq!(Preferences::open(path.clone()).theme(), Theme::Springfield);

    for bytes in [
        b"not json".as_slice(),
        br#"{"version":2,"theme":"sector-7-g"}"#,
        br#"{"version":1,"theme":"unknown"}"#,
        br#"{"version":1,"theme":"sector-7-g","removed-field":true}"#,
    ] {
        fs::write(&path, bytes).expect("write invalid preferences");
        assert_eq!(Preferences::open(path.clone()).theme(), Theme::Springfield);
    }
}

#[test]
fn only_known_themes_parse() {
    for theme in Theme::ALL {
        assert_eq!(Theme::parse(theme.as_str()), Some(*theme));
    }
    assert_eq!(Theme::parse("Evergreen Terrace"), None);
    assert_eq!(Theme::parse("unknown"), None);
}
