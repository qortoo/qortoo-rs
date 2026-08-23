use derive_more::Display;

use super::OperationBody;
use crate::operations::{MemoryMeasurable, Operation};

#[derive(Debug, Clone, Display, PartialEq, Eq)]
#[display("(delta={delta})")]
pub struct CounterIncreaseBody {
    pub delta: i64,
}

impl CounterIncreaseBody {
    pub fn new(delta: i64) -> Self {
        Self { delta }
    }
}

impl MemoryMeasurable for CounterIncreaseBody {
    fn size(&self) -> u64 {
        size_of::<i64>() as u64
    }
}

impl Operation {
    pub fn new_counter_increase(delta: i64) -> Self {
        Self::new(OperationBody::CounterIncrease(CounterIncreaseBody::new(
            delta,
        )))
    }
}

#[cfg(test)]
mod tests_counter_increase_body {
    use tracing::info;

    use super::*;

    #[test]
    fn can_display_and_debug_a_counter_increase_body() {
        let body = OperationBody::CounterIncrease(CounterIncreaseBody::new(123));
        info!("{body} vs. {body:?}");

        let display = format!("{body}");
        assert!(display.starts_with("CounterIncrease(") && display.ends_with(')'));
    }

    #[test]
    fn can_measure_a_counter_increase_body() {
        let body = OperationBody::CounterIncrease(CounterIncreaseBody::new(123));
        assert_eq!(body.size(), size_of::<i64>() as u64);
    }
}
