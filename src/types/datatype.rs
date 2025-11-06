use derive_more::Display;

/// DataType represents the kinds of Datatypes in SyncYam
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[repr(i32)]
pub enum DataType {
    #[display("Counter")]
    Counter = 0,
    #[display("Variable")]
    Variable = 1,
    #[display("Map")]
    Map = 2,
}

/// DatatypeState represents the state of a Datatype in SyncYam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum DatatypeState {
    /// The Datatype is scheduled to be created on the SyncYam server (ReadWritable).
    #[default]
    DueToCreate = 0,
    /// The Datatype is scheduled to be subscribed on the SyncYam server (ReadOnly).
    DueToSubscribe = 1,
    /// The Datatype is scheduled to be subscribed or created if it does not exist on the SyncYam server (ReadWritable).
    DueToSubscribeOrCreate = 2,
    /// The Datatype has been subscribed on the SyncYam server (ReadWritable).
    Subscribed = 3,
    /// The Datatype is scheduled to be unsubscribed from the SyncYam server (ReadOnly).
    DueToUnsubscribe = 4,
    /// The Datatype is scheduled to be deleted from the SyncYam server (ReadOnly).
    DueToDelete = 5,
    /// The Datatype is neither enabled nor synchronized with the SyncYam server (ReadOnly).
    Disabled = 6,
}

impl DatatypeState {
    /// Returns whether this state allows write operations.
    ///
    /// A datatype is writable when it is in one of these states:
    /// - `DueToCreate` - scheduled for creation
    /// - `DueToSubscribeOrCreate` - scheduled for subscription or creation
    /// - `Subscribed` - actively subscribed
    ///
    /// # Examples
    ///
    /// ```
    /// # use syncyam::DatatypeState;
    /// assert!(DatatypeState::DueToCreate.is_read_writable());
    /// assert!(DatatypeState::Subscribed.is_read_writable());
    /// assert!(!DatatypeState::DueToSubscribe.is_read_writable());
    /// assert!(!DatatypeState::Disabled.is_read_writable());
    /// ```
    pub fn is_read_writable(&self) -> bool {
        matches!(
            self,
            DatatypeState::DueToCreate
                | DatatypeState::DueToSubscribeOrCreate
                | DatatypeState::Subscribed
        )
    }

    pub fn is_readonly(&self) -> bool {
        !self.is_read_writable()
    }
}

#[cfg(test)]
mod tests_datatype {
    use rstest::rstest;

    use super::*;

    #[test]
    fn can_display_data_types() {
        assert_eq!(format!("{}", DataType::Counter), "Counter");
        assert_eq!(format!("{}", DataType::Variable), "Variable");
        assert_eq!(format!("{}", DataType::Map), "Map");
    }

    #[rstest]
    #[case::due_to_create(DatatypeState::DueToCreate, true)]
    #[case::subscribed(DatatypeState::Subscribed, true)]
    #[case::due_to_subscribe_or_create(DatatypeState::DueToSubscribeOrCreate, true)]
    #[case::due_to_subscribe(DatatypeState::DueToSubscribe, false)]
    #[case::disabled(DatatypeState::Disabled, false)]
    fn can_check_accessiblity_of_datatype_state(
        #[case] state: DatatypeState,
        #[case] expected: bool,
    ) {
        assert_eq!(state.is_read_writable(), expected);
        assert_eq!(state.is_readonly(), !expected);
    }
}
