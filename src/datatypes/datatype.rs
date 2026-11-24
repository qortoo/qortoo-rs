use crate::{
    DataType, DatatypeError, DatatypeState,
    datatypes::{handler::DatatypeHandler, transactional::TransactionalDatatype},
};

/// Shared metadata, lifecycle, synchronization, and handler methods for
/// [`Counter`](crate::Counter), [`Variable`](crate::Variable), and [`Map`](crate::Map).
///
/// Cloning a handle shares the same local value and lifecycle state. Value
/// reads are available in every lifecycle state. Local value writes require a
/// [writable state](DatatypeState::is_read_writable) and will be rejected for
/// handles built with [`DatatypeBuilder::with_readonly`](crate::DatatypeBuilder::with_readonly).
///
/// # Examples
///
/// ```
/// use qortoo::{Client, DataType, Datatype};
///
/// let client = Client::builder("doc-example", "datatype-trait")
///     .build_and_open()
///     .unwrap();
/// let map = client.create_datatype("settings").build_map().unwrap();
/// assert_eq!(map.get_key(), "settings");
/// assert_eq!(map.get_type(), DataType::Map);
/// assert_eq!(map.get_state_bytes(), 0);
/// ```
pub trait Datatype {
    /// Returns the key that identifies this datatype within its collection.
    fn get_key(&self) -> &str;
    /// Returns the kind of this datatype.
    fn get_type(&self) -> DataType;
    /// Returns the current lifecycle state.
    fn get_state(&self) -> DatatypeState;
    /// Returns the logical size in bytes of the current local CRDT state.
    ///
    /// Includes pending local changes and provisional changes during an active
    /// transaction. Counter uses 8 bytes. Variable uses its stored JSON byte
    /// length. Each Map entry uses 42 metadata bytes plus its UTF-8 key length
    /// and, for a live entry, its stored JSON byte length.
    /// Heap allocation, serialized snapshots, and buffered operations have separate sizes.
    fn get_state_bytes(&self) -> u64;
    /// Returns the server sequence recorded in this client's sync checkpoint.
    ///
    /// The checkpoint starts at `0`. A sync can advance it by more than one
    /// or leave it unchanged.
    fn get_server_version(&self) -> u64;
    /// Returns the current local transaction sequence.
    ///
    /// The sequence advances once per transaction that records at least one
    /// operation. A read during an active transaction can see the provisional
    /// sequence, which rolls back if the transaction fails.
    fn get_client_version(&self) -> u64;
    /// Returns the highest client transaction sequence acknowledged by the backend.
    ///
    /// Committed local transactions with a greater sequence are pending synchronization.
    fn get_synced_client_version(&self) -> u64;
    /// Synchronizes local changes with the connectivity backend.
    ///
    /// This method pushes pending local transactions to the server and pulls
    /// remote changes. A confirmed subscription moves the datatype to
    /// [`DatatypeState::Subscribed`]. A confirmed unsubscribe moves it to
    /// [`DatatypeState::Disabled`].
    ///
    /// # Errors
    ///
    /// Returns an error if synchronization fails, including
    /// [`DatatypeError::NotWritable`] on a disabled datatype or
    /// [`DatatypeError::Disallowed`] when called on the thread that owns this
    /// datatype's active local transaction. This includes calls through a
    /// captured original handle inside the callback. Calls from other threads
    /// exchange with the backend and wait to apply the response until the local
    /// transaction finishes.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(all(feature = "snapshot-bitcode", feature = "snapshot-zstd"))]
    /// # {
    /// use qortoo::{Client, Datatype, DatatypeState, LocalConnectivity};
    ///
    /// let connectivity = LocalConnectivity::new_arc();
    /// connectivity.set_realtime(false);
    /// let client = Client::builder("doc-example", "Datatype-sync")
    ///     .with_connectivity(connectivity)
    ///     .build_and_open()
    ///     .unwrap();
    /// let counter = client.create_datatype("key").build_counter().unwrap();
    /// counter.increase().unwrap();
    /// counter.sync().unwrap();
    /// assert_eq!(counter.get_state(), DatatypeState::Subscribed);
    /// # }
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
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError::NotWritable`] if the datatype is outside the
    /// [`Subscribed`](DatatypeState::Subscribed) state.
    fn unsubscribe(&self) -> Result<(), DatatypeError>;

    /// Registers a handler at priority `id`, replacing any handler at that priority.
    ///
    /// Handlers run in ascending priority order.
    fn set_handler(&self, id: usize, handler: DatatypeHandler);

    /// Removes the handler at priority `id` and returns whether it existed.
    fn unset_handler(&self, id: usize) -> bool;

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

    fn get_state_bytes(&self) -> u64 {
        self.get_core().get_state_bytes()
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

    fn unset_handler(&self, id: usize) -> bool {
        self.get_core().unset_handler(id)
    }

    #[cfg(test)]
    fn get_attr(&self) -> std::sync::Arc<crate::datatypes::common::Attribute> {
        self.get_core().core.attr.clone()
    }
}

#[cfg(test)]
mod tests_datatype_trait {
    use std::time::Duration;

    use tracing::instrument;

    use crate::{
        Client, DataType, DatatypeError, DatatypeState,
        datatypes::{
            common::new_attribute, datatype::Datatype, transactional::TransactionalDatatype,
        },
        utils::test_utils::{
            get_test_collection_name, get_test_func_name, get_test_ids, local_connectivity,
        },
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
        assert_eq!(data.get_state_bytes(), 8);
        assert_eq!(data.get_server_version(), 0);
        assert_eq!(data.get_client_version(), 0);
        assert_eq!(data.get_synced_client_version(), 0);
    }

    #[test]
    fn can_read_counter_and_variable_logical_state_sizes() {
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build_and_open()
            .unwrap();
        let counter = client.create_datatype("counter").build_counter().unwrap();
        let variable = client.create_datatype("variable").build_variable().unwrap();
        assert_eq!(variable.get_state_bytes(), 4);
        counter.increase().unwrap();
        variable.set_raw(b" [1, 2, 3] ").unwrap();
        assert_eq!(counter.get_state_bytes(), 8);
        assert_eq!(variable.get_state_bytes(), 11);
        assert_eq!(
            client.get_datatype("variable").unwrap().get_state_bytes(),
            11
        );
    }

    #[test]
    #[instrument]
    fn can_use_sync_method() {
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client1 = Client::builder(collection, key.clone())
            .with_connectivity(connectivity.clone())
            .build_and_open()
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
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build_and_open()
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
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build_and_open()
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
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build_and_open()
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
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let client = Client::builder(collection, get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build_and_open()
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
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let client = Client::builder(collection, get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build_and_open()
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
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let (collection, key, _) = get_test_ids!();
        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build_and_open()
            .unwrap();
        let client2 = Client::builder(collection, "client2")
            .with_connectivity(connectivity)
            .build_and_open()
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
        let connectivity = local_connectivity();
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build_and_open()
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
