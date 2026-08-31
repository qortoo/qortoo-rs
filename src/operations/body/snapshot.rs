use derive_more::Display;

use super::OperationBody;
use crate::operations::{MemoryMeasurable, Operation};

#[derive(Debug, Clone, Display, PartialEq, Eq)]
#[display("(size:{})", data.len())]
pub struct SnapshotBody {
    pub data: Box<[u8]>,
}

impl SnapshotBody {
    pub fn new(data: Box<[u8]>) -> Self {
        Self { data }
    }
}

impl MemoryMeasurable for SnapshotBody {
    fn size(&self) -> u64 {
        self.data.len() as u64
    }
}

impl Operation {
    pub fn new_snapshot(body: Box<[u8]>) -> Self {
        Self::new(OperationBody::Snapshot(SnapshotBody::new(body)))
    }
}
