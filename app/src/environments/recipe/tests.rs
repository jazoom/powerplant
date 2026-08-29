use super::{EnvironmentDraft, EnvironmentRecipe, RecipeError};

fn draft(name: &str, image: &str, script: &str) -> EnvironmentDraft {
    EnvironmentDraft {
        name: name.to_owned(),
        oci_image: image.to_owned(),
        setup_script: script.to_owned(),
    }
}

fn recipe(image: &str, script: &str) -> EnvironmentRecipe {
    EnvironmentRecipe::from_draft(&draft("Alpine Git", image, script))
        .expect("recipe")
        .1
}

#[test]
fn empty_and_oversized_values_are_rejected() {
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft(" ", "alpine/git", "")).err(),
        Some(RecipeError::Name)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "", "")).err(),
        Some(RecipeError::Image)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft(&"n".repeat(81), "alpine/git", "")).err(),
        Some(RecipeError::Name)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", &"a".repeat(513), "")).err(),
        Some(RecipeError::Image)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "alpine/git", &"a".repeat(65_537))).err(),
        Some(RecipeError::Script)
    );
}

#[test]
fn controls_local_paths_archives_and_disk_images_are_rejected() {
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("En\nv", "alpine/git", "")).err(),
        Some(RecipeError::Name)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "alpine/git\u{0007}", "")).err(),
        Some(RecipeError::Image)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "alpine/git", "echo\u{0007}")).err(),
        Some(RecipeError::Script)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "/var/images/root", "")).err(),
        Some(RecipeError::LocalPath)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "./alpine", "")).err(),
        Some(RecipeError::LocalPath)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "docker-archive:image.tar", "")).err(),
        Some(RecipeError::Archive)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "oci-archive:image.tar", "")).err(),
        Some(RecipeError::Archive)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "./disk.qcow2", "")).err(),
        Some(RecipeError::DiskImage)
    );
    assert_eq!(
        EnvironmentRecipe::from_draft(&draft("Env", "guest.raw", "")).err(),
        Some(RecipeError::DiskImage)
    );
}

#[test]
fn line_endings_normalise_before_digest() {
    let lf = recipe("alpine/git", "apk add git\n");
    let crlf = EnvironmentRecipe::from_draft(&draft("Alpine Git", "alpine/git", "apk add git\r\n"))
        .expect("crlf")
        .1;
    let cr = EnvironmentRecipe::from_draft(&draft("Alpine Git", "alpine/git", "apk add git\r"))
        .expect("cr")
        .1;
    assert_eq!(lf.setup_script, "apk add git\n");
    assert_eq!(crlf.setup_script, lf.setup_script);
    assert_eq!(cr.setup_script, lf.setup_script);
    assert_eq!(lf.version(), crlf.version());
    assert_eq!(lf.version(), cr.version());
}

#[test]
fn name_only_edits_retain_the_recipe_version() {
    let first = recipe("alpine/git", "apk add curl\n");
    let renamed = EnvironmentRecipe::from_draft(&draft("Renamed", "alpine/git", "apk add curl\n"))
        .expect("renamed")
        .1;
    assert_eq!(first.version(), renamed.version());
}

#[test]
fn image_or_script_changes_create_another_version() {
    let first = recipe("alpine/git", "");
    let image = recipe("alpine:3.20", "");
    let script = recipe("alpine/git", "apk add git\n");
    assert_ne!(first.version(), image.version());
    assert_ne!(first.version(), script.version());
}

#[test]
fn submitted_name_case_is_retained() {
    let (name, _) =
        EnvironmentRecipe::from_draft(&draft(" Alpine Git ", "alpine/git", "")).expect("name");
    assert_eq!(name, "Alpine Git");
}

#[test]
fn tabs_and_newlines_are_permitted_in_scripts() {
    let (_, recipe) =
        EnvironmentRecipe::from_draft(&draft("Env", "alpine/git", "echo\tok\n")).expect("script");
    assert_eq!(recipe.setup_script, "echo\tok\n");
}
