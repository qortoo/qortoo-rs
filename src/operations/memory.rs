pub trait MemoryMeasurable {
    fn size(&self) -> u64;
}
