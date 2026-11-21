use ubyte::ByteUnit;

pub(crate) const DEFAULT_THREAD_WORKERS: usize = 5usize;

pub(crate) const DEFAULT_MAX_MEM_SIZE_OF_PUSH_BUFFER: u64 = 100 * ByteUnit::MB.as_u64();
pub(crate) const LOWER_MAX_MEM_SIZE_OF_PUSH_BUFFER: u64 = ByteUnit::MB.as_u64();
pub(crate) const UPPER_MAX_MEM_SIZE_OF_PUSH_BUFFER: u64 = ByteUnit::GB.as_u64();

pub(crate) const DEFAULT_MAX_TRANSMISSION_BYTES: u64 = 4 * ByteUnit::MB.as_u64();
pub(crate) const DEFAULT_MAX_SNAPSHOT_BYTES: usize = 10_000_000;

pub(crate) const DEFAULT_SNAPSHOT_MAX_INPUT_BYTES: usize = 129 * ByteUnit::MiB.as_u64() as usize;
pub(crate) const DEFAULT_SNAPSHOT_MAX_RAW_BYTES: usize = 128 * ByteUnit::MiB.as_u64() as usize;
pub(crate) const DEFAULT_SNAPSHOT_MAX_MAP_ENTRIES: usize = 1_000_000;
// Writer and reader must agree on the maximum Zstd reference window (8 MiB).
pub(crate) const SNAPSHOT_MAX_ZSTD_WINDOW_LOG: u32 = 23;

pub(crate) const DEFAULT_EVENT_LOOP_TIMEOUT_MS: u64 = 100;
