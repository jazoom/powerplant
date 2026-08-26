use std::collections::BTreeMap;

use super::{SANDBOX_OWNER_LABEL, SANDBOX_OWNER_VALUE, owns_sandbox};

#[test]
fn sandbox_ownership_requires_the_power_plant_label() {
    let mut labels = BTreeMap::new();
    assert!(!owns_sandbox(&labels));

    labels.insert(SANDBOX_OWNER_LABEL.to_owned(), "another-owner".to_owned());
    assert!(!owns_sandbox(&labels));

    labels.insert(
        SANDBOX_OWNER_LABEL.to_owned(),
        SANDBOX_OWNER_VALUE.to_owned(),
    );
    assert!(owns_sandbox(&labels));
}
