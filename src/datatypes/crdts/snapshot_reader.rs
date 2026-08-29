use crate::{DatatypeError, errors::datatypes::deserialize_error};

pub(crate) struct SnapshotReader<'a> {
    remaining: &'a [u8],
    context: &'static str,
}

impl<'a> SnapshotReader<'a> {
    pub(crate) fn new(serialized: &'a [u8], context: &'static str) -> Self {
        Self {
            remaining: serialized,
            context,
        }
    }

    pub(crate) fn read_u8(&mut self, field: &str) -> Result<u8, DatatypeError> {
        Ok(self.read_array::<1>(field)?[0])
    }

    pub(crate) fn read_u64_le(&mut self, field: &str) -> Result<u64, DatatypeError> {
        Ok(u64::from_le_bytes(self.read_array::<8>(field)?))
    }

    pub(crate) fn read_bytes(
        &mut self,
        field: &str,
        length: usize,
    ) -> Result<&'a [u8], DatatypeError> {
        let Some((value, remaining)) = self.remaining.split_at_checked(length) else {
            return Err(deserialize_error(
                self.context,
                format_args!(
                    "truncated {field}: expected {length} bytes, {} remain",
                    self.remaining.len()
                ),
            ));
        };
        self.remaining = remaining;
        Ok(value)
    }

    pub(crate) fn finish(self) -> Result<(), DatatypeError> {
        if self.remaining.is_empty() {
            return Ok(());
        }
        Err(deserialize_error(
            self.context,
            format_args!("{} trailing bytes", self.remaining.len()),
        ))
    }

    fn read_array<const LENGTH: usize>(
        &mut self,
        field: &str,
    ) -> Result<[u8; LENGTH], DatatypeError> {
        let mut value = [0; LENGTH];
        value.copy_from_slice(self.read_bytes(field, LENGTH)?);
        Ok(value)
    }
}
