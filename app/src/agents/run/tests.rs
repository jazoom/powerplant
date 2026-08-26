use std::sync::Arc;

use super::RunCoordinator;
use crate::agents::AgentId;

#[test]
fn an_agent_has_one_lease() {
    let coordinator = Arc::new(RunCoordinator::new());
    let id = AgentId::generate().expect("id");
    let lease = coordinator.acquire(id).expect("lease");
    assert!(coordinator.acquire(id).is_err());
    drop(lease);
    assert!(coordinator.acquire(id).is_ok());
}
