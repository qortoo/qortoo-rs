use std::fmt::{Debug, Formatter};

use derive_more::Display;

use super::memory::MemoryMeasurable;

mod counter;
mod snapshot;
mod variable;

pub use counter::CounterIncreaseBody;
pub use snapshot::SnapshotBody;
pub use variable::VariableSetBody;

#[derive(Clone, Display, PartialEq, Eq)]
pub enum OperationBody {
    #[display("CounterIncrease{_0}")]
    CounterIncrease(CounterIncreaseBody),
    #[display("VariableSet{_0}")]
    VariableSet(VariableSetBody),
    #[display("Snapshot{_0}")]
    Snapshot(SnapshotBody),
}

impl Debug for OperationBody {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl MemoryMeasurable for OperationBody {
    fn size(&self) -> u64 {
        match self {
            OperationBody::CounterIncrease(body) => body.size(),
            OperationBody::VariableSet(body) => body.size(),
            OperationBody::Snapshot(body) => body.size(),
        }
    }
}
