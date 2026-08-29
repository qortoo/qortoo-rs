#![allow(
    dead_code,
    reason = "Variable CRDT execution wiring is implemented in the next slice"
)]

use std::{
    cmp::Ordering,
    fmt::{Debug, Formatter},
};

use crate::{DatatypeError, errors::datatypes::InternalReason, types::timestamp::Timestamp};

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct VariableState {
    value: Box<[u8]>,
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
    Applied { previous: Option<VariableState> },
    Unchanged,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct VariableCrdt {
    winning: Option<VariableState>,
}

impl VariableCrdt {
    pub(crate) fn value(&self) -> Option<&[u8]> {
        self.winning.as_ref().map(VariableState::value)
    }

    pub(crate) fn timestamp(&self) -> Option<&Timestamp> {
        self.winning.as_ref().map(VariableState::timestamp)
    }

    pub(crate) fn apply_set(
        &mut self,
        value: &[u8],
        timestamp: &Timestamp,
    ) -> Result<VariableSetOutcome, DatatypeError> {
        let Some(current) = &self.winning else {
            return Ok(self.replace(value, timestamp));
        };

        match timestamp.cmp(current.timestamp()) {
            Ordering::Greater => Ok(self.replace(value, timestamp)),
            Ordering::Less => Ok(VariableSetOutcome::Unchanged),
            Ordering::Equal if value == current.value() => Ok(VariableSetOutcome::Unchanged),
            Ordering::Equal => Err(InternalReason::ExecuteOperation(format!(
                "variable received different values for timestamp {timestamp}"
            ))
            .into_error()),
        }
    }

    fn replace(&mut self, value: &[u8], timestamp: &Timestamp) -> VariableSetOutcome {
        let previous = self.winning.replace(VariableState::new(value, timestamp));
        VariableSetOutcome::Applied { previous }
    }
}

impl Debug for VariableCrdt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VariableCrdt")
            .field("winning", &self.winning)
            .finish()
    }
}

#[cfg(test)]
mod tests_variable_crdt {
    use super::*;
    use crate::types::uid::Cuid;

    fn timestamp(lamport: u64, cuid: &str) -> Timestamp {
        Timestamp::new(lamport, &Cuid::try_from(cuid).unwrap())
    }

    #[test]
    fn can_start_a_variable_without_a_value() {
        let variable = VariableCrdt::default();

        assert_eq!(variable.value(), None);
        assert_eq!(variable.timestamp(), None);
    }

    #[test]
    fn can_apply_the_first_variable_set() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(1, "0000000000000001");

        let outcome = variable.apply_set(b"first", &timestamp).unwrap();

        assert_eq!(outcome, VariableSetOutcome::Applied { previous: None });
        assert_eq!(variable.value(), Some(b"first".as_slice()));
        assert_eq!(variable.timestamp(), Some(&timestamp));
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

        let VariableSetOutcome::Applied {
            previous: Some(previous),
        } = outcome
        else {
            panic!("expected the previous variable state");
        };
        assert_eq!(previous.value(), b"previous");
        assert_eq!(previous.timestamp(), &previous_timestamp);
        assert_eq!(variable.value(), Some(b"winning".as_slice()));
        assert_eq!(variable.timestamp(), Some(&winning_timestamp));
    }

    #[test]
    fn can_ignore_a_stale_variable_set() {
        let mut variable = VariableCrdt::default();
        let winning_timestamp = timestamp(2, "0000000000000001");
        let stale_timestamp = timestamp(1, "0000000000000002");
        variable.apply_set(b"winning", &winning_timestamp).unwrap();

        let outcome = variable.apply_set(b"stale", &stale_timestamp).unwrap();

        assert_eq!(outcome, VariableSetOutcome::Unchanged);
        assert_eq!(variable.value(), Some(b"winning".as_slice()));
        assert_eq!(variable.timestamp(), Some(&winning_timestamp));
    }

    #[test]
    fn can_ignore_a_duplicate_variable_set() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(1, "0000000000000001");
        variable.apply_set(b"same", &timestamp).unwrap();

        let outcome = variable.apply_set(b"same", &timestamp).unwrap();

        assert_eq!(outcome, VariableSetOutcome::Unchanged);
        assert_eq!(variable.value(), Some(b"same".as_slice()));
        assert_eq!(variable.timestamp(), Some(&timestamp));
    }

    #[test]
    fn can_break_variable_set_ties_by_cuid() {
        let mut variable = VariableCrdt::default();
        let lower_timestamp = timestamp(1, "0000000000000001");
        let higher_timestamp = timestamp(1, "0000000000000002");
        variable.apply_set(b"lower", &lower_timestamp).unwrap();

        let outcome = variable.apply_set(b"higher", &higher_timestamp).unwrap();

        assert!(matches!(
            outcome,
            VariableSetOutcome::Applied { previous: Some(_) }
        ));
        assert_eq!(variable.value(), Some(b"higher".as_slice()));
        assert_eq!(variable.timestamp(), Some(&higher_timestamp));
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
        assert_eq!(variable.value(), Some(b"winning-secret".as_slice()));
        assert_eq!(variable.timestamp(), Some(&timestamp));
    }
}
