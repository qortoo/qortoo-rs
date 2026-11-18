use std::cmp::Ordering;

use crate::{
    DatatypeError,
    errors::{datatypes::InternalReason, messages::datatype as msg},
    operations::{Operation, body::OperationBody},
    types::{timestamp::Timestamp, uid::Cuid},
};

/// Execution-time view of an operation and its complete precedence metadata.
///
/// An operation carries its Lamport time, while its originating CUID lives in
/// the local operation ID or the enclosing remote transaction. This context
/// combines those values without changing the persisted operation format.
/// Snapshot application bypasses this context because an initial snapshot may
/// carry Lamport zero, while modification operations must use positive values.
pub(crate) struct OperationContext<'a> {
    operation: &'a Operation,
    timestamp: Timestamp,
}

impl<'a> OperationContext<'a> {
    pub(crate) fn try_new(operation: &'a Operation, cuid: &Cuid) -> Result<Self, DatatypeError> {
        if operation.lamport == 0 {
            return Err(InternalReason::ExecuteOperation(
                msg::POSITIVE_LAMPORT_REQUIRED.to_owned(),
            )
            .into_error());
        }

        Ok(Self {
            operation,
            timestamp: Timestamp::new(operation.lamport, cuid),
        })
    }

    pub(crate) fn body(&self) -> &'a OperationBody {
        &self.operation.body
    }

    pub(crate) fn counter_input(&self) -> Result<i64, DatatypeError> {
        match self.body() {
            OperationBody::CounterIncrease(body) => Ok(body.delta),
            _ => Err(
                InternalReason::ExecuteOperation(msg::UNSUPPORTED_OPERATION.into()).into_error(),
            ),
        }
    }

    pub(crate) fn variable_input(&self) -> Result<&'a [u8], DatatypeError> {
        match self.body() {
            OperationBody::VariableSet(body) => Ok(body.value.as_ref()),
            _ => Err(
                InternalReason::ExecuteOperation(msg::UNSUPPORTED_OPERATION.into()).into_error(),
            ),
        }
    }

    pub(crate) fn map_input(&self) -> Result<(&'a str, Option<&'a [u8]>), DatatypeError> {
        match self.body() {
            OperationBody::MapPut(body) => Ok((body.key.as_str(), Some(body.value.as_ref()))),
            OperationBody::MapRemove(body) => Ok((body.key.as_str(), None)),
            _ => Err(
                InternalReason::ExecuteOperation(msg::UNSUPPORTED_OPERATION.into()).into_error(),
            ),
        }
    }

    /// Returns whether this LWW operation supersedes the current state.
    /// Equal timestamps require identical payload bytes and live/tombstone state.
    pub(crate) fn should_apply_lww(
        &self,
        current_timestamp: &Timestamp,
        current_value: Option<&[u8]>,
        incoming_value: Option<&[u8]>,
    ) -> Result<bool, DatatypeError> {
        match self.timestamp.cmp(current_timestamp) {
            Ordering::Greater => Ok(true),
            Ordering::Less => Ok(false),
            Ordering::Equal if current_value == incoming_value => Ok(false),
            Ordering::Equal => Err(InternalReason::ExecuteOperation(msg::conflicting_timestamp(
                self.timestamp(),
            ))
            .into_error()),
        }
    }

    pub(crate) fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }
}

#[cfg(test)]
mod tests_operation_context {
    use super::*;

    fn new_cuid(value: &str) -> Cuid {
        Cuid::try_from(value).unwrap()
    }

    #[test]
    fn can_create_an_operation_context_with_an_origin_cuid() {
        let mut operation = Operation::new_counter_increase(1);
        operation.set_lamport(7);
        let cuid = new_cuid("0000000000000001");

        let context = OperationContext::try_new(&operation, &cuid).unwrap();

        assert!(std::ptr::eq(context.body(), &operation.body));
        assert_eq!(context.timestamp().lamport(), 7);
        assert_eq!(context.timestamp().cuid(), &cuid);
    }

    #[test]
    fn can_reject_a_modification_operation_with_zero_lamport() {
        let operation = Operation::new_counter_increase(1);

        let error = OperationContext::try_new(&operation, &Cuid::new_nil())
            .err()
            .expect("zero Lamport must be rejected");

        assert!(matches!(&error, DatatypeError::Internal(_)));
        assert!(
            error
                .to_string()
                .contains("modification operation must use a positive Lamport timestamp")
        );
    }
}
