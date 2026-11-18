use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

use crate::{
    DatatypeError,
    errors::{datatypes::InternalReason, messages::datatype as msg},
    types::timestamp::Timestamp,
};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct TimedNode {
    pub(super) ts: Timestamp,
    pub(super) v: Option<Arc<[u8]>>,
    // shortcut: approvals remain unset until GC response validation is implemented.
    pub(super) approved_sseq_upper: Option<u64>,
}

impl TimedNode {
    pub(super) fn validate_local_timestamp(
        &self,
        timestamp: &Timestamp,
    ) -> Result<(), DatatypeError> {
        if timestamp <= &self.ts {
            return Err(
                InternalReason::ExecuteOperation(msg::LOCAL_TIMESTAMP_NOT_ADVANCED.into())
                    .into_error(),
            );
        }
        Ok(())
    }
}

impl Debug for TimedNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimedNode")
            .field("ts", &self.ts)
            .field("v_size", &self.v.as_ref().map(|value| value.len()))
            .field("approved_sseq_upper", &self.approved_sseq_upper)
            .finish()
    }
}
