use crate::{
    DataType, DatatypeError, DatatypeState,
    datatypes::{handler::DatatypeHandler, transactional::TransactionalDatatype},
};

/// The `Datatype` trait defines the common interface for all
/// conflict-free datatypes (e.g., Counter, Register, Document).
///
/// Each datatype exposes:
/// - a **key**: a unique identifier used to distinguish instances in a collection,
/// - a **type**: an enum variant of [`DataType`] describing the kind of datatype,
/// - a **state**: a [`DatatypeState`] indicating the current lifecycle/state of this datatype.
///
///
/// # Example
/// ```
/// use qortoo::Client;
/// use qortoo::{Counter, Datatype};
/// use qortoo::{DatatypeState, DataType};
/// let client = Client::builder("doc-example", "Datatype-trait").build().unwrap();
/// let counter = client.create_datatype("test-counter".to_string()).build_counter().unwrap();
/// assert_eq!(counter.get_key(), "test-counter");
/// assert_eq!(counter.get_type(), DataType::Counter);
/// assert_eq!(counter.get_state(), DatatypeState::Creating);
/// ```
pub trait Datatype {
    /// Returns a unique identifier used to distinguish instances in a collection.
    fn get_key(&self) -> &str;
    /// Returns an enum variant of [`DataType`] describing the kind of this datatype.
    fn get_type(&self) -> DataType;
    /// Returns a [`DatatypeState`] indicating the current lifecycle/status of this datatype.
    fn get_state(&self) -> DatatypeState;
    /// Returns the server-side version of this datatype.
    ///
    /// The server version is incremented each time the server acknowledges
    /// a synchronization. Returns 0 if no synchronization has occurred.
    fn get_server_version(&self) -> u64;
    /// Returns the client-side version of this datatype.
    ///
    /// The client version is incremented with each local operation.
    /// This value represents the total number of operations applied locally.
    fn get_client_version(&self) -> u64;
    /// Returns the last synchronized client version.
    ///
    /// This represents the client version that was successfully synchronized
    /// with the server. Operations with versions greater than this value
    /// are pending synchronization.
    fn get_synced_client_version(&self) -> u64;
    /// Synchronizes local changes with the connectivity backend.
    ///
    /// This method pushes local pending operations to the server and pulls
    /// remote changes. After successful synchronization, the datatype state
    /// transitions to [`DatatypeState::Subscribed`].
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError`] if the synchronization fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use qortoo::{Client, Datatype, DatatypeState, LocalConnectivity};
    ///
    /// let connectivity = LocalConnectivity::new_arc();
    /// connectivity.set_realtime(false);
    /// let client = Client::builder("doc-example", "Datatype-sync")
    ///     .with_connectivity(connectivity)
    ///     .build()
    ///     .unwrap();
    /// let counter = client.create_datatype("key").build_counter().unwrap();
    /// counter.increase().unwrap();
    /// counter.sync().unwrap();
    /// assert_eq!(counter.get_state(), DatatypeState::Subscribed);
    /// ```
    fn sync(&self) -> Result<(), DatatypeError>;

    /// Unsubscribes this datatype from the connectivity backend.
    ///
    /// This records local intent by transitioning the datatype to
    /// [`DatatypeState::Unsubscribing`]. Backend acknowledgement happens on the
    /// next push/pull: realtime connectivity can trigger it automatically, while
    /// manual connectivity requires an explicit [`sync()`](Self::sync).
    ///
    /// Once the backend confirms unsubscribe, the datatype transitions to
    /// [`DatatypeState::Disabled`] and client-managed datatypes are detached from
    /// their owning client.
    fn unsubscribe(&self) -> Result<(), DatatypeError>;

    fn set_handler(&self, id: usize, handler: DatatypeHandler);

    fn unset_handler(&self, id: usize) -> Option<DatatypeHandler>;

    #[cfg(test)]
    fn get_attr(&self) -> std::sync::Arc<crate::datatypes::common::Attribute>;
}

pub trait DatatypeBlanket {
    fn get_core(&self) -> &TransactionalDatatype;
}

impl<T> Datatype for T
where
    T: DatatypeBlanket,
{
    fn get_key(&self) -> &str {
        self.get_core().get_key()
    }

    fn get_type(&self) -> DataType {
        self.get_core().get_type()
    }

    fn get_state(&self) -> DatatypeState {
        self.get_core().get_state()
    }

    fn get_server_version(&self) -> u64 {
        self.get_core().get_server_version()
    }

    fn get_client_version(&self) -> u64 {
        self.get_core().get_client_version()
    }

    fn get_synced_client_version(&self) -> u64 {
        self.get_core().get_synced_client_version()
    }

    fn sync(&self) -> Result<(), DatatypeError> {
        self.get_core().sync()
    }

    fn unsubscribe(&self) -> Result<(), DatatypeError> {
        self.get_core().unsubscribe()
    }

    fn set_handler(&self, id: usize, handler: DatatypeHandler) {
        self.get_core().set_handler(id, handler)
    }

    fn unset_handler(&self, id: usize) -> Option<DatatypeHandler> {
        self.get_core().unset_handler(id)
    }

    #[cfg(test)]
    fn get_attr(&self) -> std::sync::Arc<crate::datatypes::common::Attribute> {
        self.get_core().attr.clone()
    }
}

#[cfg(test)]
mod tests_datatype_trait {
    use std::time::Duration;

    use tracing::instrument;

    use crate::{
        Client, DataType, DatatypeError, DatatypeState,
        connectivity::local_connectivity::LocalConnectivity,
        datatypes::{
            common::new_attribute, datatype::Datatype, transactional::TransactionalDatatype,
        },
        utils::test_utils::{get_test_collection_name, get_test_func_name, get_test_ids},
    };

    #[test]
    #[instrument]
    fn can_call_datatype_trait_methods() {
        let attr = new_attribute!(DataType::Counter);
        let key = attr.key.as_ref();
        let data = TransactionalDatatype::new_arc(
            attr.clone(),
            DatatypeState::Creating,
            Default::default(),
        );
        assert_eq!(data.get_key(), key);
        assert_eq!(data.get_type(), DataType::Counter);
        assert_eq!(data.get_state(), DatatypeState::Creating);
        assert_eq!(data.get_server_version(), 0);
        assert_eq!(data.get_client_version(), 0);
        assert_eq!(data.get_synced_client_version(), 0);
    }

    #[test]
    #[instrument]
    fn can_use_sync_method() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client1 = Client::builder(collection, key.clone())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter1 = client1.create_datatype(key).build_counter().unwrap();

        let interceptor1 = connectivity
            .get_wired_interceptor(&resource_id, &client1.get_cuid())
            .unwrap();

        // produce push_pull error
        interceptor1
            .set_after_pull(|_pull| Err(DatatypeError::SyncFailed("injected".into()).mapping()));

        assert!(matches!(
            counter1.sync().unwrap_err(),
            DatatypeError::SyncFailed(_)
        ));
        assert_eq!(counter1.get_state(), DatatypeState::Creating);

        // make a success case
        interceptor1.set_after_pull(|_pull| Ok(()));
        assert!(counter1.sync().is_ok());
        assert_eq!(counter1.get_state(), DatatypeState::Subscribed);
    }

    #[test]
    #[instrument]
    fn can_reject_unsubscribe_before_subscribed() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();
        let counter = client
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();

        assert!(matches!(
            counter.unsubscribe().unwrap_err(),
            DatatypeError::NotWritable(_)
        ));
        assert_eq!(counter.get_state(), DatatypeState::Creating);
    }

    #[test]
    #[instrument]
    fn can_mark_unsubscribing() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();
        let counter = client
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        counter.sync().unwrap();
        assert_eq!(counter.get_state(), DatatypeState::Subscribed);

        counter.unsubscribe().unwrap();

        assert_eq!(counter.get_state(), DatatypeState::Unsubscribing);
    }

    #[test]
    #[instrument]
    fn can_reject_write_and_repeated_unsubscribe_while_unsubscribing() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();
        let counter = client
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        counter.sync().unwrap();

        counter.unsubscribe().unwrap();

        assert_eq!(counter.get_state(), DatatypeState::Unsubscribing);
        assert!(matches!(
            counter.increase().unwrap_err(),
            DatatypeError::NotWritable(_)
        ));
        assert!(matches!(
            counter.unsubscribe().unwrap_err(),
            DatatypeError::NotWritable(_)
        ));
    }

    #[test]
    #[instrument]
    fn can_sync_unsubscribe_to_disabled() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let client = Client::builder(collection, get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key).build_counter().unwrap();
        counter.sync().unwrap();

        counter.unsubscribe().unwrap();
        assert_eq!(counter.get_state(), DatatypeState::Unsubscribing);
        counter.sync().unwrap();

        assert_eq!(counter.get_state(), DatatypeState::Disabled);
        assert!(
            connectivity
                .get_local_datatype_server(&resource_id)
                .is_none()
        );
    }

    #[test]
    #[instrument]
    fn can_disable_unsubscribing_on_protocol_violation() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let client = Client::builder(collection, get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key).build_counter().unwrap();
        counter.sync().unwrap();
        counter.unsubscribe().unwrap();

        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &client.get_cuid())
            .unwrap();
        interceptor.set_after_pull(|pull| {
            pull.state = DatatypeState::Subscribed;
            Ok(())
        });

        assert!(matches!(
            counter.sync().unwrap_err(),
            DatatypeError::ServerRejected(_)
        ));
        assert_eq!(counter.get_state(), DatatypeState::Disabled);
        assert!(client.get_datatype(counter.get_key()).is_none());
    }

    #[test]
    #[instrument]
    fn can_unsubscribe_with_pending_transactions() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, _) = get_test_ids!();
        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(collection, "client2")
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();
        counter1.sync().unwrap();

        let counter2 = client2.subscribe_datatype(key).build_counter().unwrap();
        counter2.sync().unwrap();
        assert_eq!(counter2.get_value(), 0);

        counter1.increase_by(7).unwrap();
        counter1.unsubscribe().unwrap();
        counter1.sync().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::Disabled);

        counter2.sync().unwrap();
        assert_eq!(counter2.get_value(), 7);
    }

    #[test]
    #[instrument]
    fn can_auto_sync_unsubscribe_in_realtime() {
        let connectivity = LocalConnectivity::new_arc();
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();
        let counter = client
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();

        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| counter.get_state() == DatatypeState::Subscribed);

        counter.unsubscribe().unwrap();

        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| counter.get_state() == DatatypeState::Disabled);
    }
}
