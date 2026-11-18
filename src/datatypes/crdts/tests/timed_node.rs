use crate::{
    datatypes::crdts::timed_node::TimedNode,
    types::{timestamp::Timestamp, uid::Cuid},
};

#[test]
fn can_require_a_newer_timestamp_for_a_local_operation() {
    let cuid = Cuid::new_nil();
    let node = TimedNode {
        ts: Timestamp::new(2, &cuid),
        v: None,
        approved_sseq_upper: None,
    };

    for lamport in [1, 2] {
        let error = node
            .validate_local_timestamp(&Timestamp::new(lamport, &cuid))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "[E202] internal error: execute operation: local operation did not advance the winning timestamp"
        );
    }
    assert!(
        node.validate_local_timestamp(&Timestamp::new(3, &cuid))
            .is_ok()
    );
}
