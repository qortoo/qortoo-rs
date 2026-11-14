use crate::{
    DatatypeError,
    datatypes::value::validate_json_value,
    errors::{datatypes::deserialize_error, messages::snapshot as msg},
    types::timestamp::{TIMESTAMP_ENCODED_LEN, Timestamp},
};

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

    pub(super) fn read_version(&mut self, expected: u8) -> Result<(), DatatypeError> {
        let version = self.read_u8(msg::VERSION_FIELD)?;
        if version != expected {
            return Err(deserialize_error(
                self.context,
                msg::unsupported_version(version),
            ));
        }
        Ok(())
    }

    pub(super) fn read_timestamp(&mut self, field: &str) -> Result<Timestamp, DatatypeError> {
        Timestamp::from_bytes(self.read_bytes(field, TIMESTAMP_ENCODED_LEN)?)
    }

    pub(super) fn read_sized(&mut self, field: &str) -> Result<&'a [u8], DatatypeError> {
        let length = usize::try_from(self.read_u64_le(field)?)
            .map_err(|_| deserialize_error(self.context, msg::LENGTH_OVERFLOW))?;
        self.read_bytes(field, length)
    }

    pub(super) fn read_json(&mut self) -> Result<&'a [u8], DatatypeError> {
        let bytes = self.read_sized(msg::JSON_PAYLOAD_FIELD)?;
        validate_json_value(bytes).map_err(|_| deserialize_error(self.context, msg::INVALID_JSON))
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
                msg::truncated_field(field, length, self.remaining.len()),
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
            msg::trailing_bytes(self.remaining.len()),
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
