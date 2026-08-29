use std::sync::Arc;

use super::AgentLeaseCoordinator;
use crate::agents::AgentId;

#[test]
fn an_agent_has_one_lease() {
    let coordinator = Arc::new(AgentLeaseCoordinator::new());
    let id = AgentId::generate().expect("id");
    let lease = coordinator.acquire(id).expect("lease");
    assert!(coordinator.acquire(id).is_err());
    drop(lease);
    assert!(coordinator.acquire(id).is_ok());
}
