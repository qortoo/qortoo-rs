use std::sync::Arc;

use tracing::instrument;

use crate::{
    DatatypeError, DatatypeState,
    datatypes::{
        common::{Attribute, ReturnType},
        crdts::Crdt,
        push_buffer::{MemoryPushBuffer, PushBuffer},
        rollback::Rollback,
    },
    errors::push_pull::ClientPushPullError,
    operations::{Operation, transaction::Transaction},
    types::{checkpoint::CheckPoint, operation_id::OperationId},
};

#[derive(Debug)]
pub struct MutableDatatype {
    pub attr: Arc<Attribute>,
    pub crdt: Crdt,
    pub state: DatatypeState,
    pub op_id: OperationId,
    pub transaction: Option<Transaction>,
    pub rollback: Rollback,
    pub push_buffer: MemoryPushBuffer,
    pub checkpoint: CheckPoint,
}

pub struct OperationalDatatype<'a> {
    pub crdt: &'a mut Crdt,
    pub op_id: &'a mut OperationId,
}

impl MutableDatatype {
    pub fn new(attr: Arc<Attribute>, state: DatatypeState) -> Self {
        let crdt = Crdt::new(attr.r#type);
        let op_id = OperationId::new_with_cuid(&attr.client_common.cuid);
        Self {
            push_buffer: MemoryPushBuffer::new(attr.option.clone()),
            rollback: Rollback::new(crdt.clone(), state, op_id.clone()),
            transaction: Default::default(),
            checkpoint: CheckPoint::default(),
            attr,
            crdt,
            state,
            op_id,
        }
    }

    #[instrument(skip_all)]
    pub fn do_rollback(&mut self) {
        self.op_id = self.rollback.op_id.clone();
        self.state = self.rollback.state;
        self.crdt = self.rollback.shadow_crdt.clone();
        self.replay_push_buffer();
    }

    pub fn end_transaction(&mut self, tag: Option<String>, committed: bool) -> bool {
        if committed {
            if let Some(mut tx) = self.transaction.take() {
                tx.set_tag(tag);
                let tx = Arc::new(tx);
                if *tx.cuid() == self.op_id.cuid {
                    if let Err(err) = self.push_buffer.enque(tx.clone()) {
                        if err == ClientPushPullError::ExceedMaxMemSize {
                            todo!("should reduce the push buffer size");
                        }
                        if err == ClientPushPullError::NonSequentialCseq {
                            unreachable!("this should not happen");
                        }
                    }
                }
                return true;
            }
        } else {
            self.do_rollback();
        }
        false
    }

    fn replay_local_operation(
        op_dt: &mut OperationalDatatype,
        op: &Operation,
        op_id: &OperationId,
    ) {
        op_dt.op_id.sync(op_id);
        let result = op_dt.crdt.execute_local_operation(op);
        if result.is_err() {
            // this cannot happen
            unreachable!()
        }
    }

    fn replay_push_buffer(&mut self) {
        for tx in self.push_buffer.iter() {
            if *tx.cuid() == self.op_id.cuid {
                let mut op_id = tx.get_op_id();
                tx.iter().for_each(|op| {
                    op_id.lamport = op.lamport;
                    let mut op_dt = OperationalDatatype {
                        crdt: &mut self.crdt,
                        op_id: &mut self.op_id,
                    };
                    Self::replay_local_operation(&mut op_dt, op, &op_id);
                });
            } else {
                // TODO: replay remote operation
            }
        }
    }

    #[instrument(skip_all)]
    pub fn execute_local_operation(
        &mut self,
        mut op: Operation,
    ) -> Result<ReturnType, DatatypeError> {
        let is_new_tx = self.transaction.is_none();
        if is_new_tx {
            self.transaction = Some(Transaction::new(&mut self.op_id));
        }
        op.set_lamport(self.op_id.next_lamport());
        let result = self.crdt.execute_local_operation(&op);
        if result.is_ok() {
            if let Some(tx) = self.transaction.as_mut() {
                tx.push_operation(op);
            }
        } else {
            if is_new_tx {
                self.op_id.prev_cseq();
                self.transaction = None;
            }
            self.op_id.prev_lamport();
        }
        result
    }
}
