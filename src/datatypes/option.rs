use crate::defaults;

pub struct DatatypeOption {
    pub max_num_of_rollback_transactions: u32,
    pub max_size_of_rollback_memory: u64,
}

impl DatatypeOption {
    pub fn new(max_num_of_rollback_transactions: u32, max_size_of_rollback_memory: u64) -> Self {
        Self {
            max_num_of_rollback_transactions,
            max_size_of_rollback_memory,
        }
    }
}

impl Default for DatatypeOption {
    fn default() -> Self {
        Self::new(
            defaults::DEFAULT_MAX_NUM_OF_ROLLBACK_TRANSACTIONS,
            defaults::DEFAULT_MAX_SIZE_OF_ROLLBACK_MEMORY,
        )
    }
}
