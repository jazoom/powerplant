use super::AssetPaths;

#[test]
fn parses_entry_paths() {
    let assets = AssetPaths::parse(
        r#"{"assets/main.ts":{"file":"assets/main.js","css":["assets/main.css"]}}"#,
    )
    .unwrap();
    assert_eq!(assets.js_path, "/static/assets/main.js");
    assert_eq!(assets.css_path, "/static/assets/main.css");
}

#[test]
fn rejects_malformed_json() {
    assert!(
        AssetPaths::parse("not json")
            .unwrap_err()
            .contains("could not parse JSON")
    );
}

#[test]
fn rejects_missing_entry_and_files() {
    assert!(
        AssetPaths::parse("{}")
            .unwrap_err()
            .contains("missing assets/main.ts")
    );
    assert!(
        AssetPaths::parse(r#"{"assets/main.ts":{}}"#)
            .unwrap_err()
            .contains("no JavaScript")
    );
}
