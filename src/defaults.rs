pub(crate) const DEFAULT_MAX_NUM_OF_ROLLBACK_TRANSACTIONS: u32 = 10_000;
pub(crate) const LOWER_MAX_NUM_OF_ROLLBACK_TRANSACTIONS: u32 = 5;
pub(crate) const UPPER_MAX_NUM_OF_ROLLBACK_TRANSACTIONS: u32 = 1_000_000;

pub(crate) const DEFAULT_MAX_SIZE_OF_ROLLBACK_MEMORY: u64 = 10 * ubyte::ByteUnit::MiB.as_u64();
pub(crate) const LOWER_MAX_SIZE_OF_ROLLBACK_MEMORY: u64 = ubyte::ByteUnit::MiB.as_u64();
pub(crate) const UPPER_MAX_SIZE_OF_ROLLBACK_MEMORY: u64 = ubyte::ByteUnit::GiB.as_u64();
