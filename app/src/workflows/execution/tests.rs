use std::sync::Arc;

use super::WorkflowExecution;

#[test]
fn one_process_holds_the_execution_lease() {
    let execution = Arc::new(WorkflowExecution::new());
    let lease = execution.acquire().expect("lease");
    assert!(execution.acquire().is_err());
    drop(lease);
    assert!(execution.acquire().is_ok());
}
