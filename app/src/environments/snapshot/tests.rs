use super::{
    EnvironmentSnapshotRepository, OciManifestDigest, SnapshotArtifactKey, SnapshotAvailability,
    SnapshotDigest, parse_sha256_digest,
};
use crate::environments::id::PreparationId;

#[test]
fn artifact_keys_are_derived_from_preparation_identifiers() {
    let id = PreparationId::generate().expect("id");
    let key = SnapshotArtifactKey::from_preparation(&id);
    assert_eq!(key.as_str(), id.as_hex());
    assert_eq!(
        SnapshotArtifactKey::parse(&id.as_hex())
            .expect("parse")
            .as_str(),
        id.as_hex()
    );
    assert!(SnapshotArtifactKey::parse("../etc").is_none());
    assert!(SnapshotArtifactKey::parse(&"A".repeat(32)).is_none());
}

#[test]
fn digest_text_must_be_canonical_lowercase_sha256() {
    let hex = "a".repeat(64);
    assert_eq!(
        SnapshotDigest::parse(&format!("sha256:{hex}"))
            .expect("ok")
            .as_str(),
        format!("sha256:{hex}")
    );
    assert!(SnapshotDigest::parse(&hex).is_none());
    assert!(SnapshotDigest::parse(&format!("SHA256:{hex}")).is_none());
    assert!(SnapshotDigest::parse(&format!("sha256:{}", "A".repeat(64))).is_none());
    assert!(SnapshotDigest::parse(&format!("sha256:{hex}/../x")).is_none());
    assert!(OciManifestDigest::parse("sha256:short").is_none());
    assert!(parse_sha256_digest("").is_none());
}

#[test]
fn artifact_paths_stay_under_the_snapshot_root() {
    let dir = tempfile::tempdir().expect("dir");
    let repository = EnvironmentSnapshotRepository::open(dir.path().to_path_buf()).expect("open");
    let id = PreparationId::generate().expect("id");
    let key = SnapshotArtifactKey::from_preparation(&id);
    let path = repository.artifact_dir(&key).expect("path");
    assert_eq!(path.parent(), Some(dir.path()));
    assert_eq!(path.file_name().unwrap().to_string_lossy(), id.as_hex());
}

#[tokio::test]
async fn missing_artifacts_are_unavailable() {
    let dir = tempfile::tempdir().expect("dir");
    let repository = EnvironmentSnapshotRepository::open(dir.path().to_path_buf()).expect("open");
    let snapshot = super::tests_support::sample_snapshot(PreparationId::generate().expect("id"));
    assert_eq!(
        repository.inspect(&snapshot).await,
        SnapshotAvailability::Missing
    );
}

#[tokio::test]
async fn in_memory_overrides_control_availability() {
    let repository = EnvironmentSnapshotRepository::in_memory();
    let snapshot = super::tests_support::sample_snapshot(PreparationId::generate().expect("id"));
    repository.mark(
        snapshot.artifact_key.clone(),
        SnapshotAvailability::Available,
    );
    assert_eq!(
        repository.inspect(&snapshot).await,
        SnapshotAvailability::Available
    );
}
