use std::collections::HashMap;

use scryer_application::IndexerRoutingPlan;

#[test]
fn indexer_routing_plan_remains_source_constructible() {
    let plan = IndexerRoutingPlan {
        entries: HashMap::new(),
    };

    assert!(plan.entries.is_empty());
}
