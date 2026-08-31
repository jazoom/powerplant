use super::ProjectForm;
use crate::projects::ProjectError;

fn form(name: &str, path: &str, revision: &str) -> ProjectForm {
    ProjectForm {
        name: name.to_owned(),
        path: path.to_owned(),
        revision: revision.to_owned(),
    }
}

#[test]
fn submitted_names_reject_empty_controls_and_bounds() {
    assert_eq!(
        form("  ", "/srv/app", "").submitted_name().err(),
        Some(ProjectError::Name)
    );
    assert_eq!(
        form("bad\nname", "/srv/app", "").submitted_name().err(),
        Some(ProjectError::Name)
    );
    assert_eq!(
        form(&"a".repeat(81), "/srv/app", "").submitted_name().err(),
        Some(ProjectError::Name)
    );
    assert_eq!(
        form("  Desk  ", "/srv/app", "").submitted_name().as_deref(),
        Ok("Desk")
    );
}

#[test]
fn submitted_paths_reject_relative_and_control_characters() {
    assert_eq!(
        form("Desk", "relative/project", "").submitted_path().err(),
        Some(ProjectError::Path)
    );
    assert_eq!(
        form("Desk", "/tmp/\nproject", "").submitted_path().err(),
        Some(ProjectError::Path)
    );
}

#[test]
fn submitted_paths_preserve_significant_whitespace() {
    let parent = tempfile::tempdir().expect("parent");
    let worktree = parent.path().join("project ");
    std::fs::create_dir(&worktree).expect("worktree");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&worktree)
            .status()
            .expect("git")
            .success()
    );
    let raw = worktree.to_string_lossy();
    assert_eq!(
        form("Desk", &raw, "").submitted_path().expect("path"),
        worktree.canonicalize().expect("canonical")
    );
}

#[test]
fn revision_parser_accepts_positive_decimals() {
    assert_eq!(form("", "", "1").revision(), Ok(1));
    assert_eq!(form("", "", &u32::MAX.to_string()).revision(), Ok(u32::MAX));
    assert_eq!(form("", "", " 2 ").revision(), Ok(2));
}

#[test]
fn revision_parser_rejects_malformed_and_excessive_values() {
    for value in ["", "0", "01", "1a", "4294967296", "1.0", "-1"] {
        assert_eq!(
            form("", "", value).revision(),
            Err(super::REVISION_MESSAGE),
            "{value}"
        );
    }
}
