use std::fmt::{Debug, Display, Formatter};
#[cfg(test)]
use std::sync::Arc;

use crate::{
    operations::{MemoryMeasurable, Operation},
    types,
    types::{operation_id::OperationId, uid::Cuid},
};

const TRANSACTION_CONSTANT_SIZE: u64 = (size_of::<Vec<Operation>>() // operations
    + types::uid::UID_LEN // cuid
    + size_of::<Option<String>>() // tag
    + size_of::<u64>() // cseq
    + size_of::<u64>() // sseq
    + size_of::<bool>())  // event
    as u64;

#[derive(PartialEq, Eq)]
pub struct Transaction {
    cuid: Cuid,
    cseq: u64,
    sseq: u64,
    tag: Option<String>,
    event: bool,
    operations: Vec<Operation>,
}

impl Transaction {
    pub fn new(op_id: &mut OperationId) -> Self {
        Self {
            cuid: op_id.cuid.clone(),
            cseq: op_id.next_cseq(),
            sseq: 0,
            tag: None,
            event: false,
            operations: vec![],
        }
    }

    pub fn cseq(&self) -> u64 {
        self.cseq
    }

    pub fn cuid(&self) -> &Cuid {
        &self.cuid
    }

    pub fn get_op_id(&self) -> OperationId {
        let mut op_id = OperationId::new_with_cuid(&self.cuid);
        op_id.cseq = self.cseq;
        op_id
    }

    pub fn set_tag(&mut self, tag: Option<String>) {
        self.tag = tag;
    }

    pub fn set_event(&mut self, event: bool) {
        self.event = event;
    }

    pub fn push_operation(&mut self, op: Operation) {
        self.operations.push(op);
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Operation> {
        self.operations.iter()
    }

    #[cfg(test)]
    pub fn new_arc_for_test(cuid: &Cuid, cseq: u64) -> Arc<Self> {
        let operations = Vec::new();
        Arc::new(Self {
            cuid: cuid.clone(),
            cseq,
            sseq: 0,
            tag: None,
            event: false,
            operations,
        })
    }
}

impl Debug for Transaction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl Display for Transaction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let tag_arg = match &self.tag {
            Some(tag) => format!("🔖:{tag}"),
            None => String::new(),
        };
        let event_arg = if self.event { " ✅ " } else { " " };
        let mut lamport_arg = String::from("[]");
        if !self.operations.is_empty() {
            let first = &self.operations[0];
            if self.operations.len() > 1 {
                if let Some(last) = self.operations.last() {
                    lamport_arg = format!("[{}-{}]", first, last)
                }
            } else {
                lamport_arg = format!("[{}]", first)
            }
        }
        f.write_fmt(format_args!(
            "TX({}{}{}:{}:{}:{})",
            tag_arg, event_arg, self.cuid, self.cseq, self.sseq, lamport_arg,
        ))
    }
}

impl MemoryMeasurable for Transaction {
    fn size(&self) -> u64 {
        let op_size: u64 = self.operations.iter().map(|op| op.size()).sum();
        let tag_size: u64 = match &self.tag {
            Some(s) => s.len() as u64,
            None => 0,
        };
        TRANSACTION_CONSTANT_SIZE + tag_size + op_size
    }
}

#[cfg(test)]
mod tests_transaction {
    use tracing::info;

    use super::{OperationId, TRANSACTION_CONSTANT_SIZE, Transaction};
    use crate::operations::{MemoryMeasurable, Operation};

    #[test]
    fn can_debug_and_display_transaction() {
        let mut op_id = OperationId::new();
        let mut tx = Transaction::new(&mut op_id);
        info!("{tx}");
        tx.set_tag(Some("tag1".to_string()));
        tx.set_event(true);
        info!("{tx}");
        let mut op1 = Operation::new_counter_increase(1);
        op1.lamport = 1;
        let mut op2 = Operation::new_counter_increase(2);
        op2.lamport = 2;
        tx.operations.push(op1);
        info!("{tx:?}");
        tx.set_event(false);
        tx.operations.push(op2);
        info!("{tx}");

        let op_id_tx = tx.get_op_id();
        info!("{op_id_tx}");
        assert_eq!(op_id, op_id_tx);
    }

    #[test]
    fn can_measure_transaction_size() {
        let mut op_id = OperationId::new();
        let mut tx = Transaction::new(&mut op_id);
        assert_eq!(tx.size(), TRANSACTION_CONSTANT_SIZE);
        tx.set_tag(Some("1234567890".to_string()));
        assert_eq!(tx.size(), TRANSACTION_CONSTANT_SIZE + 10);
        let op = Operation::new_counter_increase(1);
        tx.push_operation(op.clone());
        assert_eq!(tx.size(), TRANSACTION_CONSTANT_SIZE + 10 + op.size());
        tx.push_operation(op.clone());
        assert_eq!(tx.size(), TRANSACTION_CONSTANT_SIZE + 10 + op.size() * 2);
    }
}
