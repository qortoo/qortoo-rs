use ubyte::ByteUnit;

pub(crate) const DEFAULT_THREAD_WORKERS: usize = 4usize;

pub(crate) const DEFAULT_MAX_MEM_SIZE_OF_PUSH_BUFFER: u64 = 100 * ByteUnit::MB.as_u64();
pub(crate) const LOWER_MAX_MEM_SIZE_OF_PUSH_BUFFER: u64 = ByteUnit::MB.as_u64();
pub(crate) const UPPER_MAX_MEM_SIZE_OF_PUSH_BUFFER: u64 = ByteUnit::GB.as_u64();

pub(crate) const DEFAULT_MAX_TRANSMISSION_SIZE: u64 = 4 * ByteUnit::MB.as_u64();
