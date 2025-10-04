use crate::defaults::{
    DEFAULT_MAX_MEM_SIZE_OF_PUSH_BUFFER, LOWER_MAX_MEM_SIZE_OF_PUSH_BUFFER,
    UPPER_MAX_MEM_SIZE_OF_PUSH_BUFFER,
};

#[derive(Debug, Clone)]
pub struct DatatypeOption {
    pub max_mem_size_of_push_buffer: u64,
}

impl DatatypeOption {
    pub fn new(max_size_of_push_buffer: u64) -> Self {
        Self {
            max_mem_size_of_push_buffer: max_size_of_push_buffer.clamp(
                LOWER_MAX_MEM_SIZE_OF_PUSH_BUFFER,
                UPPER_MAX_MEM_SIZE_OF_PUSH_BUFFER,
            ),
        }
    }
}

impl Default for DatatypeOption {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_MEM_SIZE_OF_PUSH_BUFFER)
    }
}

#[cfg(test)]
mod tests_datatype_option {
    use tracing::info;

    use crate::{datatypes::option::DatatypeOption, defaults::LOWER_MAX_MEM_SIZE_OF_PUSH_BUFFER};

    #[test]
    fn can_use_datatype_option() {
        let option = DatatypeOption::new(LOWER_MAX_MEM_SIZE_OF_PUSH_BUFFER - 100);
        info!("{option:?}");
        assert_eq!(
            option.max_mem_size_of_push_buffer,
            LOWER_MAX_MEM_SIZE_OF_PUSH_BUFFER
        );
    }
}
