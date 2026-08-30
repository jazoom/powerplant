use std::collections::BTreeSet;

use super::recovered_cleanup_record;
use crate::workflows::run::AttemptCleanupRecord;
use crate::workflows::workspace::WorkspaceRecovery;
use crate::workflows::{AttemptId, RunId};

#[test]
fn startup_recovery_records_each_remaining_resource() {
    let run = RunId::generate().expect("run");
    let attempt = AttemptId::generate().expect("attempt");
    let workspace = WorkspaceRecovery {
        run,
        attempt,
        remains: true,
    };
    let mut guests = BTreeSet::new();

    let mut remaining_runs = BTreeSet::new();
    assert_eq!(
        recovered_cleanup_record(true, &guests, &remaining_runs, &[], run, attempt),
        AttemptCleanupRecord::Complete
    );
    assert_eq!(
        recovered_cleanup_record(true, &guests, &remaining_runs, &[workspace], run, attempt),
        AttemptCleanupRecord::Orphaned {
            sandbox: false,
            workspace: true,
        }
    );
    guests.insert(attempt);
    assert_eq!(
        recovered_cleanup_record(true, &guests, &remaining_runs, &[workspace], run, attempt),
        AttemptCleanupRecord::Orphaned {
            sandbox: true,
            workspace: true,
        }
    );
    remaining_runs.insert(run);
    assert_eq!(
        recovered_cleanup_record(true, &BTreeSet::new(), &remaining_runs, &[], run, attempt,),
        AttemptCleanupRecord::Orphaned {
            sandbox: true,
            workspace: false,
        }
    );
    assert_eq!(
        recovered_cleanup_record(false, &BTreeSet::new(), &BTreeSet::new(), &[], run, attempt),
        AttemptCleanupRecord::Orphaned {
            sandbox: true,
            workspace: false,
        }
    );
}
