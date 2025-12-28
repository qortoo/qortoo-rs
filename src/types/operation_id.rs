use std::{
    cmp::Ordering,
    fmt::{Debug, Display, Formatter},
};

use crate::types::uid::Cuid;

#[derive(PartialEq, Default, Clone)]
pub struct OperationId {
    pub lamport: u64,
    pub cuid: Cuid,
    pub cseq: u64,
}

impl OperationId {
    pub fn new_with_cuid(cuid: &Cuid) -> Self {
        Self {
            cuid: cuid.clone(),
            ..Default::default()
        }
    }

    pub fn new() -> Self {
        Self {
            cuid: Cuid::new_nil(),
            ..Default::default()
        }
    }

    pub fn next_lamport(&mut self) -> u64 {
        self.lamport += 1;
        self.lamport
    }

    pub fn prev_lamport(&mut self) -> u64 {
        if self.lamport > 0 {
            self.lamport -= 1;
        }
        self.lamport
    }

    pub fn next_cseq(&mut self) -> u64 {
        self.cseq += 1;
        self.cseq
    }

    pub fn prev_cseq(&mut self) -> u64 {
        if self.cseq > 0 {
            self.cseq -= 1;
        }
        self.cseq
    }

    pub fn sync(&mut self, other: &Self) {
        self.lamport = self.lamport.max(other.lamport);
        if other.cuid == self.cuid {
            self.cseq = self.cseq.max(other.cseq);
        }
    }
}

impl Debug for OperationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.to_string().as_str())
    }
}

impl Display for OperationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}:{}:{}", self.lamport, self.cuid, self.cseq))
    }
}

impl PartialOrd for OperationId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.lamport > other.lamport {
            return Some(Ordering::Greater);
        } else if self.lamport < other.lamport {
            return Some(Ordering::Less);
        }
        self.cuid.partial_cmp(&other.cuid)
    }
}

#[cfg(test)]
mod tests_operation_id {
    use tracing::info;

    use super::*;

    #[test]
    fn can_new_and_display_operation_id() {
        let op_id = OperationId::new_with_cuid(&Cuid::new_nil());
        info!("{op_id:?}");
        assert_eq!(op_id.to_string(), "0:0000000000000000:0");
    }

    #[test]
    fn can_next_rollback_compare_operation_ids() {
        let mut op_id1 = OperationId::new();
        let op_id2 = OperationId::new();
        assert_eq!(op_id1, op_id2);
        op_id1.next_lamport();
        assert!(op_id1 > op_id2);
        assert!(op_id2 < op_id1);
        op_id1.prev_lamport();
        assert_eq!(op_id1, op_id2);
        assert_eq!(op_id1, op_id1);
    }

    #[test]
    fn can_test_operation_id_methods() {
        let mut op_id1 = OperationId::new();
        let mut op_id2 = OperationId::new();
        op_id1.prev_cseq();

        assert_eq!(op_id1, op_id2);
        op_id1.prev_lamport();
        assert_eq!(op_id1, op_id2);

        op_id2.next_cseq();
        op_id1.sync(&op_id2);
        assert_eq!(op_id1, op_id2);
        op_id1.prev_cseq();

        assert_eq!(op_id1.partial_cmp(&op_id2).unwrap(), Ordering::Equal);
    }
}
