use std::{collections::VecDeque, fmt::Display, sync::Arc};

use crate::{
    DatatypeError,
    datatypes::option::DatatypeOption,
    errors::datatypes::InternalReason,
    operations::{MemoryMeasurable, transaction::Transaction},
};

#[allow(dead_code)]
pub trait PushBuffer {
    fn enqueue(&mut self, tx: Arc<Transaction>) -> Result<(), DatatypeError>;
    fn get_pushing_transactions(
        &mut self,
        cseq: u64,
        max_mem_size: u64,
    ) -> Result<(Vec<Arc<Transaction>>, u64), DatatypeError>;
    fn deque(&mut self, upto_cseq: u64) -> Vec<Arc<Transaction>>;
}

#[derive(Debug)]
pub struct MemoryPushBuffer {
    transaction: VecDeque<Arc<Transaction>>,
    pub mem_size: u64,
    pub option: Arc<DatatypeOption>,
    pub first_cseq: u64,
    pub last_cseq: u64,
}

impl MemoryPushBuffer {
    pub fn new(option: Arc<DatatypeOption>) -> Self {
        Self {
            transaction: VecDeque::new(),
            option,
            mem_size: 0u64,
            first_cseq: 0u64,
            last_cseq: 0u64,
        }
    }

    /// Returns an iterator over the transactions in the push buffer
    pub fn iter(&self) -> impl Iterator<Item = &Arc<Transaction>> {
        self.transaction.iter()
    }

    #[allow(dead_code)]
    fn need_to_deque(tx: Option<&Arc<Transaction>>, cseq: u64) -> bool {
        if let Some(tx) = tx {
            if tx.cseq <= cseq {
                return true;
            }
        }
        false
    }
}

impl PushBuffer for MemoryPushBuffer {
    fn enqueue(&mut self, tx: Arc<Transaction>) -> Result<(), DatatypeError> {
        if self.last_cseq != 0 && self.last_cseq + 1 != tx.cseq {
            return Err(InternalReason::NonSequentialCseq.into_error());
        }
        if self.mem_size + tx.size() > self.option.max_mem_size_of_push_buffer {
            return Err(DatatypeError::PushBufferExceededMaxMemSize);
        }
        if self.first_cseq == 0 {
            self.first_cseq = tx.cseq;
        }
        self.last_cseq = tx.cseq;
        self.mem_size += tx.size();
        self.transaction.push_back(tx);
        Ok(())
    }

    fn get_pushing_transactions(
        &mut self,
        cseq: u64,
        max_mem_size: u64,
    ) -> Result<(Vec<Arc<Transaction>>, u64), DatatypeError> {
        let mut popped = vec![];
        if cseq == 0 || cseq < self.first_cseq {
            return Err(InternalReason::GetPushingTransactions.into_error());
        }

        let mut total_size: u64 = 0;
        let index = (cseq - self.first_cseq) as usize;
        if self.transaction.len() <= index {
            return Ok((popped, total_size));
        }

        for i in index..self.transaction.len() {
            let tx = self.transaction.get(i).unwrap().clone();
            if total_size + tx.size() > max_mem_size {
                break;
            }
            total_size += tx.size();
            popped.push(tx);
        }
        Ok((popped, total_size))
    }

    fn deque(&mut self, upto_cseq: u64) -> Vec<Arc<Transaction>> {
        let mut ret = Vec::new();
        if upto_cseq < self.first_cseq {
            return ret;
        }
        if upto_cseq > self.last_cseq {
            ret = self.transaction.drain(..).collect();
            self.mem_size = 0;
            self.first_cseq = 0;
            self.last_cseq = 0;
            return ret;
        }
        loop {
            if !Self::need_to_deque(self.transaction.front(), upto_cseq) {
                break;
            }
            let tx = self.transaction.pop_front().unwrap();
            self.mem_size -= tx.size();
            ret.push(tx);

            self.first_cseq = if let Some(front) = self.transaction.front() {
                front.cseq
            } else {
                self.last_cseq = 0;
                0
            };
        }
        ret
    }
}

impl Display for MemoryPushBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PushBuffer(mem_size: {}, cseq: #{}-#{} [{}])",
            self.mem_size,
            self.first_cseq,
            self.last_cseq,
            self.transaction.len(),
        )
    }
}

#[cfg(test)]
mod tests_push_buffer {
    use std::sync::Arc;

    use tracing::{info, instrument};

    use crate::{
        DatatypeError,
        datatypes::{
            option::DatatypeOption,
            push_buffer::{MemoryPushBuffer, PushBuffer},
        },
        operations::{MemoryMeasurable, transaction::Transaction},
        types::operation_id::OperationId,
    };

    #[test]
    #[instrument]
    fn can_enque_from_push_buffer() {
        const MAX_SIZE: u64 = 1_000_000;
        let option = Arc::new(DatatypeOption::new(MAX_SIZE));
        let mut push_buffer = MemoryPushBuffer::new(option);
        assert_eq!(push_buffer.mem_size, 0);
        assert_eq!(push_buffer.first_cseq, 0);
        assert_eq!(push_buffer.last_cseq, 0);

        let mut op_id = OperationId::new();
        let cseq = op_id.next_cseq();
        let tx = Arc::new(Transaction::new(&op_id.cuid, cseq));
        let tx_size = tx.size();
        assert!(push_buffer.enqueue(tx).is_ok());
        assert_eq!(push_buffer.mem_size, tx_size);
        assert_eq!(push_buffer.first_cseq, 1);
        assert_eq!(push_buffer.last_cseq, 1);

        for _ in 0..9 {
            let cseq = op_id.next_cseq();
            let tx = Arc::new(Transaction::new(&op_id.cuid, cseq));
            assert!(push_buffer.enqueue(tx).is_ok());
        }
        assert_eq!(push_buffer.mem_size, tx_size * 10);
        assert_eq!(push_buffer.first_cseq, 1);
        assert_eq!(push_buffer.last_cseq, 10);

        let mut op_id2 = OperationId::new();
        let cseq2 = op_id2.next_cseq();
        let tx_not_sequential = Arc::new(Transaction::new(&op_id2.cuid, cseq2));
        let result = push_buffer.enqueue(tx_not_sequential);
        assert!(matches!(result.unwrap_err(), DatatypeError::Internal(_)));

        loop {
            let cseq = op_id.next_cseq();
            let tx = Arc::new(Transaction::new(&op_id.cuid, cseq));
            if push_buffer.mem_size + tx.size() > MAX_SIZE {
                assert_eq!(
                    push_buffer.enqueue(tx).unwrap_err(),
                    DatatypeError::PushBufferExceededMaxMemSize
                );
                break;
            }
            assert!(push_buffer.enqueue(tx).is_ok());
        }
    }

    #[test]
    #[instrument]
    fn can_get_after_and_deque_from_push_buffer() {
        const MAX_PUSH_SIZE: u64 = 1_000_000;

        let option = Arc::new(DatatypeOption::default());
        let mut push_buffer = MemoryPushBuffer::new(option);
        let mut op_id = OperationId::new();
        let cseq = op_id.next_cseq();
        let tx = Arc::new(Transaction::new(&op_id.cuid, cseq));
        let tx_size = tx.size();
        assert!(push_buffer.enqueue(tx).is_ok());
        for _ in 1..100 {
            let cseq = op_id.next_cseq();
            let tx = Arc::new(Transaction::new(&op_id.cuid, cseq));
            assert!(push_buffer.enqueue(tx).is_ok());
        }
        assert_eq!(push_buffer.mem_size, tx_size * 100);
        assert_eq!(push_buffer.first_cseq, 1);
        assert_eq!(push_buffer.last_cseq, 100);

        let (push_transactions, push_tx_size) = push_buffer
            .get_pushing_transactions(50, MAX_PUSH_SIZE)
            .unwrap();
        info!("push_buffer: {push_buffer} {push_tx_size}");
        assert_eq!(push_transactions.len(), 51);
        assert_eq!(push_tx_size, tx_size * 51);
        assert_eq!(push_transactions.first().unwrap().cseq, 50);

        let (push_transactions, push_tx_size) = push_buffer
            .get_pushing_transactions(50, tx_size * 10)
            .unwrap();
        assert_eq!(push_transactions.len(), 10);
        assert_eq!(push_tx_size, tx_size * 10);
        assert_eq!(push_transactions.first().unwrap().cseq, 50);

        let (push_transactions, push_tx_size) = push_buffer
            .get_pushing_transactions(101, MAX_PUSH_SIZE)
            .unwrap();
        assert_eq!(push_transactions.len(), 0);
        assert_eq!(push_tx_size, 0);

        assert_eq!(50, push_buffer.deque(50).len());
        info!("push_buffer: {push_buffer}");
        assert_eq!(0, push_buffer.deque(0).len());
        assert_eq!(50, push_buffer.deque(101).len());
        info!("push_buffer: {push_buffer}");

        assert_eq!(push_buffer.mem_size, 0);
        assert_eq!(push_buffer.first_cseq, 0);
        assert_eq!(push_buffer.last_cseq, 0);
    }
}
