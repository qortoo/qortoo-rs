use std::{
    cmp::Ordering,
    fmt::{Debug, Display, Formatter},
    sync::Arc,
};

use serde::Deserialize;

use super::{snapshot_codec::SnapshotCodec, snapshot_reader::SnapshotReader};
use crate::{
    DatatypeError,
    datatypes::{
        common::ReturnType,
        crdts::{LocalOperationOutcome, RollbackAction},
    },
    errors::datatypes::{InternalReason, deserialize_error},
    operations::body::OperationBody,
    types::{
        operation_context::OperationContext,
        timestamp::{TIMESTAMP_ENCODED_LEN, Timestamp},
    },
};

const SNAPSHOT_VERSION: u8 = 1;
const SNAPSHOT_CONTEXT: &str = "variable crdt";
const INITIAL_VALUE: &[u8] = b"null";

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct VariableState {
    value: Arc<[u8]>,
    timestamp: Timestamp,
}

impl VariableState {
    fn new(value: &[u8], timestamp: &Timestamp) -> Self {
        Self {
            value: value.into(),
            timestamp: timestamp.clone(),
        }
    }

    pub(crate) fn value(&self) -> &[u8] {
        &self.value
    }

    pub(crate) fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }
}

impl Debug for VariableState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VariableState")
            .field("value_size", &self.value.len())
            .field("timestamp", &self.timestamp)
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VariableSetOutcome {
    Applied { previous: VariableState },
    Unchanged,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VariableRollbackAction {
    Restore { previous: VariableState },
}

#[derive(Clone, PartialEq, Eq)]
pub struct VariableCrdt {
    winning: VariableState,
}

impl Default for VariableCrdt {
    fn default() -> Self {
        Self {
            winning: VariableState::new(INITIAL_VALUE, &Timestamp::initial()),
        }
    }
}

impl VariableCrdt {
    pub(crate) fn value(&self) -> &[u8] {
        self.winning.value()
    }

    pub(crate) fn timestamp(&self) -> &Timestamp {
        self.winning.timestamp()
    }

    fn apply_set(
        &mut self,
        value: &[u8],
        timestamp: &Timestamp,
    ) -> Result<VariableSetOutcome, DatatypeError> {
        match timestamp.cmp(self.winning.timestamp()) {
            Ordering::Greater => Ok(self.replace(value, timestamp)),
            Ordering::Less => Ok(VariableSetOutcome::Unchanged),
            Ordering::Equal if value == self.winning.value() => Ok(VariableSetOutcome::Unchanged),
            Ordering::Equal => Err(InternalReason::ExecuteOperation(format!(
                "variable received different values for timestamp {timestamp}"
            ))
            .into_error()),
        }
    }

    pub(crate) fn execute_local_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<LocalOperationOutcome, DatatypeError> {
        let OperationBody::VariableSet(body) = &context.operation().body else {
            return Err(InternalReason::ExecuteOperation(
                "variable cannot execute this local operation".to_owned(),
            )
            .into_error());
        };

        let VariableSetOutcome::Applied { previous } =
            self.apply_set(&body.value, context.timestamp())?
        else {
            return Err(InternalReason::ExecuteOperation(
                "local variable set did not advance the winning timestamp".to_owned(),
            )
            .into_error());
        };
        let previous_value = Arc::clone(&previous.value);

        Ok(LocalOperationOutcome::new(
            ReturnType::Variable(previous_value),
            RollbackAction::Variable(VariableRollbackAction::Restore { previous }),
        ))
    }

    pub(crate) fn execute_remote_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<(), DatatypeError> {
        let OperationBody::VariableSet(body) = &context.operation().body else {
            return Err(InternalReason::ExecuteOperation(
                "variable cannot execute this remote operation".to_owned(),
            )
            .into_error());
        };

        self.apply_set(&body.value, context.timestamp())?;
        Ok(())
    }

    pub(crate) fn apply_rollback_action(
        &mut self,
        action: VariableRollbackAction,
    ) -> Result<(), DatatypeError> {
        match action {
            VariableRollbackAction::Restore { previous } => {
                self.winning = previous;
                Ok(())
            }
        }
    }

    fn replace(&mut self, value: &[u8], timestamp: &Timestamp) -> VariableSetOutcome {
        let previous = std::mem::replace(&mut self.winning, VariableState::new(value, timestamp));
        VariableSetOutcome::Applied { previous }
    }
}

impl SnapshotCodec for VariableCrdt {
    fn encode_snapshot(&self) -> Box<[u8]> {
        let mut serialized = Vec::new();
        serialized.push(SNAPSHOT_VERSION);
        serialized.extend_from_slice(&self.winning.timestamp.to_bytes());
        let value_length = u64::try_from(self.winning.value.len())
            .expect("variable snapshot payload length exceeds u64");
        serialized.extend_from_slice(&value_length.to_le_bytes());
        serialized.extend_from_slice(&self.winning.value);
        serialized.into_boxed_slice()
    }

    fn decode_snapshot(snapshot: &[u8]) -> Result<Self, DatatypeError> {
        let mut reader = SnapshotReader::new(snapshot, SNAPSHOT_CONTEXT);
        let version = reader.read_u8("version")?;
        if version != SNAPSHOT_VERSION {
            return Err(deserialize_error(
                SNAPSHOT_CONTEXT,
                format_args!("unsupported version {version}"),
            ));
        }

        let timestamp = reader.read_bytes("winning timestamp", TIMESTAMP_ENCODED_LEN)?;
        let timestamp = Timestamp::from_bytes(timestamp)?;
        let value_length = reader.read_u64_le("JSON length")?;
        let value_length = usize::try_from(value_length).map_err(|_| {
            deserialize_error(SNAPSHOT_CONTEXT, "JSON length does not fit this platform")
        })?;
        let value = reader.read_bytes("JSON payload", value_length)?;
        validate_snapshot_json(value)?;
        validate_snapshot_state(value, &timestamp)?;
        let winning = VariableState::new(value, &timestamp);

        reader.finish()?;
        Ok(Self { winning })
    }
}

fn validate_snapshot_json(value: &[u8]) -> Result<(), DatatypeError> {
    let mut deserializer = serde_json::Deserializer::from_slice(value);
    serde::de::IgnoredAny::deserialize(&mut deserializer).map_err(|error| {
        deserialize_error(
            SNAPSHOT_CONTEXT,
            format_args!("invalid JSON payload: {error}"),
        )
    })?;
    deserializer.end().map_err(|error| {
        deserialize_error(
            SNAPSHOT_CONTEXT,
            format_args!("invalid JSON payload: {error}"),
        )
    })
}

fn validate_snapshot_state(value: &[u8], timestamp: &Timestamp) -> Result<(), DatatypeError> {
    if timestamp.lamport() > 0 {
        return Ok(());
    }
    if timestamp.is_initial() && value == INITIAL_VALUE {
        return Ok(());
    }
    Err(deserialize_error(
        SNAPSHOT_CONTEXT,
        "Lamport zero is reserved for the initial null state",
    ))
}

impl Debug for VariableCrdt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VariableCrdt")
            .field("winning", &self.winning)
            .finish()
    }
}

impl Display for VariableCrdt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(size:{}, timestamp:{})",
            self.value().len(),
            self.timestamp()
        )
    }
}

#[cfg(test)]
mod tests_variable_crdt {
    use super::*;
    use crate::{
        operations::Operation,
        types::uid::{Cuid, UID_LEN},
    };

    fn timestamp(lamport: u64, cuid: &str) -> Timestamp {
        Timestamp::new(lamport, &Cuid::try_from(cuid).unwrap())
    }

    fn variable_set_operation(value: &[u8], lamport: u64) -> Operation {
        let mut operation = Operation::new_variable_set(value.into());
        operation.set_lamport(lamport);
        operation
    }

    fn serialized_snapshot(lamport: u64, cuid: [u8; UID_LEN], value: &[u8]) -> Vec<u8> {
        let mut serialized = vec![SNAPSHOT_VERSION];
        serialized.extend_from_slice(&lamport.to_le_bytes());
        serialized.extend_from_slice(&cuid);
        serialized.extend_from_slice(&(value.len() as u64).to_le_bytes());
        serialized.extend_from_slice(value);
        serialized
    }

    #[test]
    fn can_start_a_variable_with_the_initial_null_state() {
        let variable = VariableCrdt::default();

        assert_eq!(variable.value(), b"null");
        assert!(variable.timestamp().is_initial());
    }

    #[test]
    fn can_apply_the_first_variable_set() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(1, "0000000000000001");

        let outcome = variable.apply_set(b"first", &timestamp).unwrap();

        let VariableSetOutcome::Applied { previous } = outcome else {
            panic!("expected an applied variable set");
        };
        assert_eq!(previous.value(), b"null");
        assert!(previous.timestamp().is_initial());
        assert_eq!(variable.value(), b"first");
        assert_eq!(variable.timestamp(), &timestamp);
    }

    #[test]
    fn can_apply_an_explicit_null_over_the_initial_null_value() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(1, "0000000000000001");

        let outcome = variable.apply_set(b"null", &timestamp).unwrap();

        let VariableSetOutcome::Applied { previous } = outcome else {
            panic!("expected an applied variable set");
        };
        assert_eq!(previous.value(), b"null");
        assert!(previous.timestamp().is_initial());
        assert_eq!(variable.value(), b"null");
        assert_eq!(variable.timestamp(), &timestamp);
    }

    #[test]
    fn can_apply_a_newer_variable_set_and_return_the_previous_state() {
        let mut variable = VariableCrdt::default();
        let previous_timestamp = timestamp(1, "0000000000000001");
        let winning_timestamp = timestamp(2, "0000000000000001");
        variable
            .apply_set(b"previous", &previous_timestamp)
            .unwrap();

        let outcome = variable.apply_set(b"winning", &winning_timestamp).unwrap();

        let VariableSetOutcome::Applied { previous } = outcome else {
            panic!("expected the previous variable state");
        };
        assert_eq!(previous.value(), b"previous");
        assert_eq!(previous.timestamp(), &previous_timestamp);
        assert_eq!(variable.value(), b"winning");
        assert_eq!(variable.timestamp(), &winning_timestamp);
    }

    #[test]
    fn can_ignore_a_stale_variable_set() {
        let mut variable = VariableCrdt::default();
        let winning_timestamp = timestamp(2, "0000000000000001");
        let stale_timestamp = timestamp(1, "0000000000000002");
        variable.apply_set(b"winning", &winning_timestamp).unwrap();

        let outcome = variable.apply_set(b"stale", &stale_timestamp).unwrap();

        assert_eq!(outcome, VariableSetOutcome::Unchanged);
        assert_eq!(variable.value(), b"winning");
        assert_eq!(variable.timestamp(), &winning_timestamp);
    }

    #[test]
    fn can_ignore_a_duplicate_variable_set() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(1, "0000000000000001");
        variable.apply_set(b"same", &timestamp).unwrap();

        let outcome = variable.apply_set(b"same", &timestamp).unwrap();

        assert_eq!(outcome, VariableSetOutcome::Unchanged);
        assert_eq!(variable.value(), b"same");
        assert_eq!(variable.timestamp(), &timestamp);
    }

    #[test]
    fn can_break_variable_set_ties_by_cuid() {
        let mut variable = VariableCrdt::default();
        let lower_timestamp = timestamp(1, "0000000000000001");
        let higher_timestamp = timestamp(1, "0000000000000002");
        variable.apply_set(b"lower", &lower_timestamp).unwrap();

        let outcome = variable.apply_set(b"higher", &higher_timestamp).unwrap();

        assert!(matches!(outcome, VariableSetOutcome::Applied { .. }));
        assert_eq!(variable.value(), b"higher");
        assert_eq!(variable.timestamp(), &higher_timestamp);
    }

    #[test]
    fn can_reject_different_values_for_the_same_variable_timestamp() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(1, "0000000000000001");
        variable.apply_set(b"winning-secret", &timestamp).unwrap();

        let error = variable
            .apply_set(b"conflicting-secret", &timestamp)
            .unwrap_err();

        assert!(matches!(&error, DatatypeError::Internal(_)));
        assert!(error.to_string().contains("different values for timestamp"));
        assert!(!error.to_string().contains("secret"));
        assert_eq!(variable.value(), b"winning-secret");
        assert_eq!(variable.timestamp(), &timestamp);
    }

    #[test]
    fn can_execute_and_rollback_local_variable_sets_in_reverse_order() {
        let mut variable = VariableCrdt::default();
        let cuid = Cuid::try_from("0000000000000001").unwrap();
        let first_operation = variable_set_operation(b"first", 1);
        let first_context = OperationContext::try_new(&first_operation, &cuid).unwrap();

        let first_outcome = variable.execute_local_operation(&first_context).unwrap();
        let (first_return, first_action) = first_outcome.into_parts();
        let RollbackAction::Variable(first_action) = first_action else {
            panic!("expected a variable rollback action");
        };
        let ReturnType::Variable(first_previous_value) = first_return else {
            panic!("expected the initial variable value");
        };
        let VariableRollbackAction::Restore {
            previous: first_previous_state,
        } = &first_action;
        assert_eq!(first_previous_value.as_ref(), b"null");
        assert!(Arc::ptr_eq(
            &first_previous_value,
            &first_previous_state.value
        ));
        assert!(first_previous_state.timestamp().is_initial());

        let second_operation = variable_set_operation(b"second", 2);
        let second_context = OperationContext::try_new(&second_operation, &cuid).unwrap();
        let second_outcome = variable.execute_local_operation(&second_context).unwrap();
        let (second_return, second_action) = second_outcome.into_parts();
        let RollbackAction::Variable(second_action) = second_action else {
            panic!("expected a variable rollback action");
        };
        let ReturnType::Variable(previous_value) = second_return else {
            panic!("expected the previous variable value");
        };
        let VariableRollbackAction::Restore {
            previous: previous_state,
        } = &second_action;
        assert_eq!(previous_value.as_ref(), b"first");
        assert!(Arc::ptr_eq(&previous_value, &previous_state.value));
        assert_eq!(variable.value(), b"second");
        assert_eq!(variable.timestamp(), second_context.timestamp());

        variable.apply_rollback_action(second_action).unwrap();
        assert_eq!(variable.value(), b"first");
        assert_eq!(variable.timestamp(), first_context.timestamp());

        variable.apply_rollback_action(first_action).unwrap();
        assert_eq!(variable.value(), b"null");
        assert!(variable.timestamp().is_initial());
    }

    #[test]
    fn can_execute_remote_variable_sets_with_lww_precedence() {
        let mut variable = VariableCrdt::default();
        let cuid = Cuid::try_from("0000000000000001").unwrap();
        let winning_operation = variable_set_operation(b"winning", 2);
        let winning_context = OperationContext::try_new(&winning_operation, &cuid).unwrap();
        variable.execute_remote_operation(&winning_context).unwrap();

        let stale_operation = variable_set_operation(b"stale", 1);
        let stale_context = OperationContext::try_new(&stale_operation, &cuid).unwrap();
        variable.execute_remote_operation(&stale_context).unwrap();

        assert_eq!(variable.value(), b"winning");
        assert_eq!(variable.timestamp(), winning_context.timestamp());
    }

    #[test]
    fn can_reject_a_local_variable_set_without_a_newer_timestamp() {
        let mut variable = VariableCrdt::default();
        let cuid = Cuid::try_from("0000000000000001").unwrap();
        let winning_timestamp = Timestamp::new(2, &cuid);
        variable.apply_set(b"winning", &winning_timestamp).unwrap();
        let stale_operation = variable_set_operation(b"winning", 1);
        let stale_context = OperationContext::try_new(&stale_operation, &cuid).unwrap();

        let error = variable
            .execute_local_operation(&stale_context)
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("did not advance the winning timestamp")
        );
        assert_eq!(variable.value(), b"winning");
        assert_eq!(variable.timestamp(), &winning_timestamp);
    }

    #[test]
    fn can_reject_non_variable_operations() {
        let mut variable = VariableCrdt::default();
        let mut operation = Operation::new_counter_increase(1);
        operation.set_lamport(1);
        let context = OperationContext::try_new(&operation, &Cuid::default()).unwrap();

        assert!(variable.execute_local_operation(&context).is_err());
        assert!(variable.execute_remote_operation(&context).is_err());
        assert_eq!(variable.value(), b"null");
        assert!(variable.timestamp().is_initial());
    }

    #[test]
    fn can_round_trip_an_initial_null_variable_snapshot() {
        let variable = VariableCrdt::default();

        let serialized = variable.encode_snapshot();
        let decoded = VariableCrdt::decode_snapshot(&serialized).unwrap();

        assert_eq!(serialized[0], SNAPSHOT_VERSION);
        assert_eq!(&serialized[1..9], &0_u64.to_le_bytes());
        assert_eq!(&serialized[9..25], b"0000000000000000");
        assert_eq!(&serialized[25..33], &4_u64.to_le_bytes());
        assert_eq!(&serialized[33..], b"null");
        assert_eq!(decoded, variable);
    }

    #[test]
    fn can_round_trip_a_variable_snapshot_with_its_winning_timestamp() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(7, "0000000000000001");
        let value = br#"{"name":"qortoo"}"#;
        variable.apply_set(value, &timestamp).unwrap();

        let serialized = variable.encode_snapshot();
        let decoded = VariableCrdt::decode_snapshot(&serialized).unwrap();

        assert_eq!(serialized[0], SNAPSHOT_VERSION);
        assert_eq!(&serialized[1..9], &7_u64.to_le_bytes());
        assert_eq!(&serialized[9..25], b"0000000000000001");
        assert_eq!(&serialized[25..33], &(value.len() as u64).to_le_bytes());
        assert_eq!(&serialized[33..], value);
        assert_eq!(decoded, variable);
    }

    #[test]
    fn can_preserve_the_real_timestamp_of_an_explicit_null_set() {
        let explicit_timestamp = timestamp(7, "0000000000000001");
        let serialized = serialized_snapshot(7, *b"0000000000000001", b"null");
        let variable = VariableCrdt::decode_snapshot(&serialized).unwrap();

        assert_eq!(variable.value(), b"null");
        assert_eq!(variable.timestamp(), &explicit_timestamp);
        assert!(!variable.timestamp().is_initial());
    }

    #[test]
    fn can_reject_malformed_variable_snapshots() {
        let valid_cuid = *b"0000000000000001";
        let mut invalid_utf8_cuid = valid_cuid;
        invalid_utf8_cuid[0] = 0xff;
        let mut invalid_format_cuid = valid_cuid;
        invalid_format_cuid[0] = b'(';

        let nil_cuid = *b"0000000000000000";
        let mut truncated_payload = serialized_snapshot(7, valid_cuid, b"null");
        truncated_payload[25..33].copy_from_slice(&5_u64.to_le_bytes());
        let mut trailing_payload = serialized_snapshot(7, valid_cuid, b"null");
        trailing_payload.push(0);

        let cases = [
            ("empty", Vec::new(), "truncated version"),
            (
                "missing lamport",
                vec![SNAPSHOT_VERSION],
                "truncated winning timestamp",
            ),
            (
                "unsupported version",
                vec![SNAPSHOT_VERSION + 1],
                "unsupported version",
            ),
            (
                "truncated CUID",
                serialized_snapshot(7, valid_cuid, b"null")[..9].to_vec(),
                "truncated winning timestamp",
            ),
            (
                "invalid UTF-8 CUID",
                serialized_snapshot(7, invalid_utf8_cuid, b"null"),
                "deserialize: uid: not valid UTF-8",
            ),
            (
                "invalid CUID format",
                serialized_snapshot(7, invalid_format_cuid, b"null"),
                "deserialize: uid: invalid format",
            ),
            (
                "truncated payload",
                truncated_payload,
                "truncated JSON payload",
            ),
            (
                "invalid JSON",
                serialized_snapshot(7, valid_cuid, b"not-json"),
                "invalid JSON payload",
            ),
            (
                "non-null initial value",
                serialized_snapshot(0, nil_cuid, b"false"),
                "reserved for the initial null state",
            ),
            (
                "non-nil initial CUID",
                serialized_snapshot(0, valid_cuid, b"null"),
                "reserved for the initial null state",
            ),
            ("trailing payload bytes", trailing_payload, "trailing bytes"),
        ];

        for (case, serialized, expected_error) in cases {
            let error = VariableCrdt::decode_snapshot(&serialized).unwrap_err();
            assert!(matches!(&error, DatatypeError::Internal(_)), "{case}");
            assert!(
                error.to_string().contains(expected_error),
                "{case}: {error}"
            );
        }
    }
}
