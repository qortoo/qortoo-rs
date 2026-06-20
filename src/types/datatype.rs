use derive_more::Display;

/// Identifies the type of conflict-free datatype.
///
/// Each variant represents a different CRDT implementation with specific
/// conflict resolution semantics.
///
/// # Examples
///
/// ```
/// use qortoo::{Client, DataType, Datatype};
///
/// let client = Client::builder("doc-example", "datatype-test").build().unwrap();
/// let counter = client.create_datatype("my-counter").build_counter().unwrap();
/// assert_eq!(counter.get_type(), DataType::Counter);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[repr(i32)]
pub enum DataType {
    /// CRDT counter supporting increment/decrement operations
    #[display("Counter")]
    Counter = 0,
    /// CRDT variable (planned)
    #[display("Variable")]
    Variable = 1,
    /// CRDT map (planned)
    #[display("Map")]
    Map = 2,
}

/// Represents the lifecycle state and write-access control of a datatype.
///
/// Each state determines whether the datatype allows write operations based on
/// its synchronization lifecycle. This works in combination with the explicit
/// read-only flag set via [`crate::DatatypeBuilder::with_readonly`].
///
/// **Write Access Control:**
/// A datatype is writable only when BOTH conditions are met:
/// 1. The state allows writing (via [`is_read_writable()`](Self::is_read_writable))
/// 2. The explicit read-only flag is NOT set
///
/// # Examples
///
/// ```
/// use qortoo::{Client, DatatypeState, Datatype};
///
/// let client = Client::builder("doc-example", "state-test").build().unwrap();
///
/// // Creating state allows writing
/// let counter1 = client.create_datatype("c1").build_counter().unwrap();
/// assert_eq!(counter1.get_state(), DatatypeState::Creating);
/// assert!(counter1.increase().is_ok());
///
/// // Subscribing state prevents writing (state-based)
/// let counter2 = client.subscribe_datatype("c2").build_counter().unwrap();
/// assert_eq!(counter2.get_state(), DatatypeState::Subscribing);
/// assert!(counter2.increase().is_err());
///
/// // Explicit read-only flag prevents writing (flag-based)
/// let counter3 = client.create_datatype("c3").with_readonly().build_counter().unwrap();
/// assert_eq!(counter3.get_state(), DatatypeState::Creating);
/// assert!(counter3.increase().is_err()); // read-only despite writable state
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, strum_macros::Display)]
#[repr(i32)]
pub enum DatatypeState {
    /// The datatype is scheduled to be created and subscribed on the server (writable).
    #[default]
    Creating = 0,
    /// The datatype is scheduled to be subscribed on the server (read-only).
    Subscribing = 1,
    /// The datatype is scheduled to be subscribed or created if it doesn't exist (writable).
    SubscribingOrCreating = 2,
    /// The datatype has been successfully subscribed on the server (writable).
    Subscribed = 3,
    /// The datatype is scheduled to be unsubscribed from the server (read-only).
    Unsubscribing = 4,
    /// The datatype is scheduled to be deleted from the server (read-only).
    Deleting = 5,
    /// The datatype is neither enabled nor synchronized with the server (read-only).
    Disabled = 6,
}

impl DatatypeState {
    /// Returns whether this state allows write operations.
    ///
    /// A datatype is writable when it is in one of these states:
    /// - `Creating` - scheduled for creation
    /// - `SubscribingOrCreating` - scheduled for subscription or creation
    /// - `Subscribed` - actively subscribed
    ///
    /// **Important:** This method only checks the lifecycle state. The actual
    /// write access is controlled by BOTH this state AND the explicit read-only
    /// flag. Use the datatype's write methods to verify complete write access.
    ///
    /// # Examples
    ///
    /// ```
    /// # use qortoo::DatatypeState;
    /// assert!(DatatypeState::Creating.is_read_writable());
    /// assert!(DatatypeState::Subscribed.is_read_writable());
    /// assert!(!DatatypeState::Subscribing.is_read_writable());
    /// assert!(!DatatypeState::Disabled.is_read_writable());
    /// ```
    pub fn is_read_writable(&self) -> bool {
        matches!(
            self,
            DatatypeState::Creating
                | DatatypeState::SubscribingOrCreating
                | DatatypeState::Subscribed
        )
    }

    /// Returns whether this state prevents write operations.
    ///
    /// This is the inverse of [`is_read_writable()`](Self::is_read_writable).
    ///
    /// # Examples
    ///
    /// ```
    /// # use qortoo::DatatypeState;
    /// assert!(!DatatypeState::Creating.is_readonly());
    /// assert!(DatatypeState::Subscribing.is_readonly());
    /// assert!(DatatypeState::Disabled.is_readonly());
    /// ```
    pub fn is_readonly(&self) -> bool {
        !self.is_read_writable()
    }
}

#[cfg(test)]
mod tests_datatype {
    use rstest::rstest;
    use tracing::instrument;

    use super::*;
    use crate::{
        Client, Datatype,
        utils::test_utils::{get_test_collection_name, get_test_func_name},
    };

    #[test]
    fn can_display_data_types() {
        assert_eq!(format!("{}", DataType::Counter), "Counter");
        assert_eq!(format!("{}", DataType::Variable), "Variable");
        assert_eq!(format!("{}", DataType::Map), "Map");
    }

    #[rstest]
    #[case::creating(DatatypeState::Creating, true)]
    #[case::subscribed(DatatypeState::Subscribed, true)]
    #[case::subscribing_or_creating(DatatypeState::SubscribingOrCreating, true)]
    #[case::subscribing(DatatypeState::Subscribing, false)]
    #[case::disabled(DatatypeState::Disabled, false)]
    fn can_check_accessibility_of_datatype_state(
        #[case] state: DatatypeState,
        #[case] expected: bool,
    ) {
        assert_eq!(state.is_read_writable(), expected);
        assert_eq!(state.is_readonly(), !expected);
    }

    #[test]
    #[instrument]
    fn can_not_write_when_readonly() {
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build()
            .unwrap();

        // Creating state allows writing
        let counter1 = client.create_datatype("c1").build_counter().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::Creating);
        assert!(counter1.increase().is_ok());

        // Subscribing state prevents writing (state-based)
        let counter2 = client.subscribe_datatype("c2").build_counter().unwrap();
        assert_eq!(counter2.get_state(), DatatypeState::Subscribing);
        assert!(counter2.increase().is_err());

        // Explicit read-only flag prevents writing (flag-based)
        let counter3 = client
            .create_datatype("c3")
            .with_readonly()
            .build_counter()
            .unwrap();
        assert_eq!(counter3.get_state(), DatatypeState::Creating);
        assert!(counter3.increase().is_err()); // read-only despite writable state
    }
}
