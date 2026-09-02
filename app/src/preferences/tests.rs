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
fn saved_theme_names_load_as_their_replacements() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("preferences.json");

    for (saved, replacement) in [
        ("springfield-light", Theme::Springfield),
        ("springfield-dark", Theme::EvergreenTerrace),
        ("springfield-dark-3", Theme::EvergreenTerrace),
        ("birch", Theme::Leftorium),
        ("springfield-elementary", Theme::Leftorium),
        ("midnight", Theme::Stonecutters),
        ("nuclear-dusk", Theme::Sector7G),
    ] {
        fs::write(&path, format!(r#"{{"version":1,"theme":"{saved}"}}"#))
            .expect("write old preferences");
        assert_eq!(Preferences::open(path.clone()).theme(), replacement);
    }
}

#[test]
fn missing_or_invalid_files_default_to_light() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("preferences.json");
    assert_eq!(Preferences::open(path.clone()).theme(), Theme::Springfield);

    for bytes in [
        b"not json".as_slice(),
        br#"{"version":2,"theme":"springfield-dark"}"#,
        br#"{"version":1,"theme":"unknown"}"#,
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
