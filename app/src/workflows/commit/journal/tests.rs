use super::*;

impl CommitJournals {
    pub(crate) fn in_memory() -> Self {
        let hold = tempfile::tempdir().expect("journals");
        let root = hold.path().to_path_buf();
        storage::ensure_private_dir(&root).expect("dir");
        Self {
            root,
            _hold: Some(hold),
        }
    }
}
