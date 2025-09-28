use std::sync::Arc;

use tracing::instrument;

use crate::{
    DatatypeError, DatatypeState,
    datatypes::{
        common::{Attribute, ReturnType},
        crdts::Crdt,
        rollback::Rollback,
    },
    operations::{Operation, transaction::Transaction},
    types::operation_id::OperationId,
};

#[derive(Debug)]
pub struct MutableDatatype {
    #[allow(dead_code)]
    attr: Arc<Attribute>,
    pub crdt: Crdt,
    pub state: DatatypeState,
    pub op_id: OperationId,
    pub transaction: Option<Transaction>,
    pub rollback: Rollback,
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
            attr,
            crdt: crdt.clone(),
            state,
            op_id: op_id.clone(),
            transaction: Default::default(),
            rollback: Rollback::new(crdt, state, op_id.clone()),
        }
    }

    #[instrument(skip_all)]
    pub fn do_rollback(&mut self) {
        self.op_id = self.rollback.op_id.clone();
        self.state = self.rollback.state;
        self.crdt = self.rollback.shadow_crdt.clone();
    }

    pub fn end_transaction(&mut self, tag: Option<String>, committed: bool) {
        if committed {
            if let Some(mut tx) = self.transaction.take() {
                tx.set_tag(tag);
                let tx = Arc::new(tx);
                self.commit_transaction_on_rollback(tx.clone());
            }
        } else {
            self.do_rollback();
        }
    }

    fn replay_local_operation(
        op_dt: &mut OperationalDatatype,
        op: &Operation,
        op_id: &OperationId,
    ) -> Result<ReturnType, DatatypeError> {
        op_dt.op_id.sync(op_id);
        let result = op_dt.crdt.execute_local_operation(op);
        if result.is_err() {
            // this cannot happen
            unreachable!()
        }
        result
    }

    fn commit_transaction_on_rollback(&mut self, tx: Arc<Transaction>) {
        if *tx.cuid() == self.op_id.cuid {
            let mut op_id = tx.get_op_id();
            tx.iter().for_each(|op| {
                op_id.lamport = op.lamport;
                let mut op_dt = self.rollback.get_operational_datatype();
                Self::replay_local_operation(&mut op_dt, op, &op_id).unwrap();
            });
        } else {
            // replay remote operation
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
