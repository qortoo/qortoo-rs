use std::{collections::BTreeMap, sync::Arc};

use dyn_fmt::AsStrFormatExt;
use parking_lot::RwLock;

use crate::{
    DataType, DatatypeBuilder, DatatypeError, DatatypeHandler, DatatypeState, IntoString,
    clients::{common::ClientCommon, datatype_manager::DatatypeManager},
    connectivity::{Connectivity, null_connectivity::NullConnectivity},
    datatypes::{datatype_set::DatatypeSet, option::DatatypeOption},
    errors::clients::{CLIENT_ERROR_MSG_COLLECTION_NAME, ClientError},
    utils::name_validator::is_valid_collection_name,
};

/// A builder for constructing a [`Client`].
///
/// Use [`Client::builder`] to start, then call [`ClientBuilder::build`]
/// to obtain a ready-to-use client instance.
///
/// # Examples
/// ```
/// use qortoo::Client;
/// let client = Client::builder("doc-example", "ClientBuilder-test").build().unwrap();
/// assert_eq!(client.get_collection(), "doc-example");
/// assert_eq!(client.get_alias(), "ClientBuilder-test");
/// ```
pub struct ClientBuilder {
    collection: String,
    alias: String,
    connectivity: Arc<dyn Connectivity>,
}

impl ClientBuilder {
    /// Finalizes the builder and returns a new [`Client`].
    ///
    /// It initializes client metadata and datatype management structures.
    ///
    /// # Errors
    /// Returns [`ClientError::InvalidCollectionName`] if the collection name is invalid.
    pub fn build(self) -> Result<Client, ClientError> {
        if !is_valid_collection_name(&self.collection) {
            return Err(ClientError::InvalidCollectionName(
                CLIENT_ERROR_MSG_COLLECTION_NAME.format(&[self.collection]),
            ));
        }

        let common =
            ClientCommon::new_arc(self.collection.into(), self.alias.into(), self.connectivity);
        let datatype_manager = Arc::new(RwLock::new(DatatypeManager::new(common.clone())));
        common.set_datatype_manager(Arc::downgrade(&datatype_manager));
        Ok(Client {
            datatype_manager,
            common,
        })
    }

    /// Sets a custom connectivity backend for synchronization.
    ///
    /// By default, [`Client`] uses [`NullConnectivity`](crate::connectivity::null_connectivity::NullConnectivity),
    /// which is a no-op implementation. Use this method to provide a real
    /// connectivity backend for distributed synchronization.
    ///
    /// # Arguments
    ///
    /// * `connectivity` - An `Arc`-wrapped implementation of the [`Connectivity`] trait
    ///
    /// # Examples
    ///
    /// ```
    /// use qortoo::{Client, LocalConnectivity};
    ///
    /// let connectivity = LocalConnectivity::new_arc();
    /// let client = Client::builder("collection", "alias")
    ///     .with_connectivity(connectivity)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn with_connectivity(mut self, connectivity: Arc<dyn Connectivity>) -> Self {
        self.connectivity = connectivity;
        self
    }
}

/// Facade for creating and subscribing to Qortoo datatypes.
///
/// A `Client` is scoped by a logical `collection` and an `alias` that
/// are propagated into tracing metadata and used to associate created
/// datatypes with their owner.
///
/// Use [`Client::builder`] to construct a client, and the `create_datatype` / `subscribe_datatype` / `subscribe_or_create_datatype`
/// helpers to get specific datatypes.
pub struct Client {
    common: Arc<ClientCommon>,
    datatype_manager: Arc<RwLock<DatatypeManager>>,
}

impl Client {
    /// Returns a ClientBuilder to construct a new client with
    /// the given `collection` and `alias`.
    ///
    /// # Examples
    /// ```
    /// use qortoo::Client;
    /// let client = Client::builder("col", "alias").build().unwrap();
    /// assert_eq!(client.get_alias(), "alias");
    /// ```
    pub fn builder(collection: impl IntoString, alias: impl IntoString) -> ClientBuilder {
        ClientBuilder {
            collection: collection.into(),
            alias: alias.into(),
            connectivity: Arc::new(NullConnectivity::new()),
        }
    }

    pub(crate) fn do_subscribe_or_create_datatype(
        &self,
        key: String,
        r#type: DataType,
        state: DatatypeState,
        option: DatatypeOption,
        is_readonly: bool,
        handlers: BTreeMap<usize, DatatypeHandler>,
    ) -> Result<DatatypeSet, ClientError> {
        self.datatype_manager.write().subscribe_or_create_datatype(
            &key,
            r#type,
            state,
            option,
            is_readonly,
            handlers,
        )
    }

    /// Returns an existing datatype by `key`, if it has been created or
    /// subscribed via this client.
    pub fn get_datatype(&self, key: &str) -> Option<DatatypeSet> {
        self.datatype_manager.read().get_datatype(key)
    }

    /// Unsubscribes the datatype identified by `key` from this client.
    ///
    /// This is a key-based convenience API over [`Datatype::unsubscribe`](crate::Datatype::unsubscribe):
    /// it marks the datatype as `DueToUnsubscribe` and returns the handle in that state.
    /// The client manager entry is removed only after the backend confirms `Disabled` via
    /// the auto-detach hook — not immediately — so a sync failure leaves the datatype
    /// reachable and retryable through the manager.
    ///
    /// The caller drives the datatype to `Disabled`:
    /// - with manual connectivity, call `sync()` on the returned handle,
    /// - with realtime connectivity, the event loop handles it automatically.
    pub fn unsubscribe_datatype(&self, key: &str) -> Result<DatatypeSet, DatatypeError> {
        let datatype = self.get_datatype(key).ok_or_else(|| {
            DatatypeError::Disallowed(format!("Datatype '{key}' is not managed by this client"))
        })?;

        datatype.unsubscribe()?;
        Ok(datatype)
    }

    /// Returns the collection name this client is associated with.
    pub fn get_collection(&self) -> &str {
        &self.common.collection
    }

    /// Returns the alias (application/client name) for this client.
    pub fn get_alias(&self) -> &str {
        &self.common.alias
    }

    /// Get `DatatypeBuilder` to subscribe a `Datatype` identified by `key`.
    ///
    /// The `Datatype` built by this builder will be marked
    /// with [`DatatypeState::DueToSubscribe`].
    pub fn subscribe_datatype(&self, key: impl IntoString) -> DatatypeBuilder<'_> {
        DatatypeBuilder::new(self, key.into(), DatatypeState::DueToSubscribe)
    }

    /// Get `DatatypeBuilder` to create a `Datatype` identified by `key`.
    ///
    /// The `Datatype` built by this builder will be marked
    /// with [`DatatypeState::DueToCreate`].
    pub fn create_datatype(&self, key: impl IntoString) -> DatatypeBuilder<'_> {
        DatatypeBuilder::new(self, key.into(), DatatypeState::DueToCreate)
    }

    /// Get `DatatypeBuilder` to subscribe or create a `Datatype` identified by `key`.
    ///
    /// The `Datatype` built by this builder will be marked
    /// with [`DatatypeState::DueToSubscribeOrCreate`].
    pub fn subscribe_or_create_datatype(&self, key: impl IntoString) -> DatatypeBuilder<'_> {
        DatatypeBuilder::new(self, key.into(), DatatypeState::DueToSubscribeOrCreate)
    }

    #[cfg(test)]
    pub fn get_cuid(&self) -> crate::types::uid::Cuid {
        self.common.cuid.clone()
    }
}

#[cfg(test)]
mod tests_client {
    use tracing::instrument;

    use crate::{
        Datatype, DatatypeState, LocalConnectivity,
        clients::client::Client,
        datatypes::datatype::DatatypeBlanket,
        utils::test_utils::{get_test_collection_name, get_test_func_name},
    };

    #[test]
    fn can_assert_send_and_sync_traits() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Client>();
    }

    #[test]
    #[instrument]
    fn can_build_client() {
        let client = Client::builder("collection1", "alias1").build().unwrap();
        assert_eq!(client.get_collection(), "collection1");
        assert_eq!(client.get_alias(), "alias1");
    }

    #[test]
    #[instrument]
    fn can_use_counter_from_client() {
        let lc = LocalConnectivity::new_arc();
        lc.set_realtime(false);
        let client1 = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(lc)
            .build()
            .unwrap();

        assert!(client1.get_datatype("k1").is_none());
        let counter1 = client1.subscribe_datatype("k1").build_counter().unwrap();
        assert_eq!(DatatypeState::DueToSubscribe, counter1.get_state());
        assert!(client1.get_datatype("k1").is_some());

        let client2 = Client::builder(get_test_collection_name!(), get_test_collection_name!())
            .build()
            .unwrap();
        let counter2 = client2.create_datatype("k1").build_counter().unwrap();
        assert_eq!(DatatypeState::DueToCreate, counter2.get_state());

        let client3 = Client::builder(get_test_collection_name!(), get_test_collection_name!())
            .build()
            .unwrap();
        let counter3 = client3
            .subscribe_or_create_datatype("k1")
            .build_counter()
            .unwrap();
        assert_eq!(counter3.get_state(), DatatypeState::DueToSubscribeOrCreate);
    }

    #[test]
    #[instrument]
    fn can_reject_invalid_collection_names() {
        // Empty collection name
        assert!(Client::builder("", "alias").build().is_err());

        // Too long (> 47 characters)
        assert!(Client::builder("a".repeat(48), "alias").build().is_err());

        // Starts with digit
        assert!(Client::builder("1hello", "alias").build().is_err());

        // Starts with system.
        assert!(Client::builder("system.reserved", "alias").build().is_err());

        // Contains .system.
        assert!(Client::builder("my.system.db", "alias").build().is_err());

        // Invalid special character
        assert!(Client::builder("hello@world", "alias").build().is_err());

        // Valid names should succeed
        assert!(Client::builder("valid_name", "alias").build().is_ok());
        assert!(Client::builder("my-collection", "alias").build().is_ok());
        assert!(Client::builder("system", "alias").build().is_ok());
        assert!(Client::builder("hello.system", "alias").build().is_ok());
    }

    #[test]
    #[instrument]
    fn can_auto_detach_after_create_failure_disables_datatype() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let collection = get_test_collection_name!();
        let key = get_test_func_name!();
        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(collection, "client2")
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        client1.create_datatype(key.clone()).build_counter().unwrap().sync().unwrap();

        let counter = client2.create_datatype(key.clone()).build_counter().unwrap();
        assert!(client2.get_datatype(&key).is_some());
        assert!(matches!(
            counter.sync().unwrap_err(),
            crate::DatatypeError::FailedToCreate(_)
        ));

        assert_eq!(counter.get_state(), DatatypeState::Disabled);
        assert!(client2.get_datatype(&key).is_none());
    }

    #[test]
    #[instrument]
    fn can_auto_detach_after_subscribe_failure_disables_datatype() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();
        let key = get_test_func_name!();

        let counter = client.subscribe_datatype(key.clone()).build_counter().unwrap();
        assert!(client.get_datatype(&key).is_some());
        assert!(matches!(
            counter.sync().unwrap_err(),
            crate::DatatypeError::FailedToSubscribe(_)
        ));

        assert_eq!(counter.get_state(), DatatypeState::Disabled);
        assert!(client.get_datatype(&key).is_none());
    }

    #[test]
    #[instrument]
    fn can_unsubscribe_datatype_from_client() {
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
        let old_core = counter.get_core() as *const _;

        let removed = client.unsubscribe_datatype(counter.get_key()).unwrap();
        assert_eq!(removed.get_state(), DatatypeState::DueToUnsubscribe);
        assert!(client.get_datatype(counter.get_key()).is_some());

        counter.sync().unwrap();
        assert_eq!(removed.get_state(), DatatypeState::Disabled);
        assert_eq!(counter.get_state(), DatatypeState::Disabled);
        assert!(client.get_datatype(counter.get_key()).is_none());

        let counter2 = client
            .subscribe_datatype(counter.get_key())
            .build_counter()
            .unwrap();
        let new_core = counter2.get_core() as *const _;
        assert_ne!(old_core, new_core);
    }

    #[test]
    #[instrument]
    fn can_auto_detach_after_datatype_unsubscribe_sync() {
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
        let key = counter.get_key().to_owned();
        let old_core = counter.get_core() as *const _;

        counter.unsubscribe().unwrap();
        counter.sync().unwrap();

        assert_eq!(counter.get_state(), DatatypeState::Disabled);
        assert!(client.get_datatype(&key).is_none());

        let counter2 = client.subscribe_datatype(&key).build_counter().unwrap();
        let new_core = counter2.get_core() as *const _;
        assert_ne!(old_core, new_core);
    }

    #[test]
    #[instrument]
    fn can_auto_detach_after_datatype_unsubscribe_in_realtime() {
        let connectivity = LocalConnectivity::new_arc();
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        let counter = client
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        awaitility::at_most(std::time::Duration::from_secs(1))
            .poll_interval(std::time::Duration::from_micros(100))
            .until(|| counter.get_state() == DatatypeState::Subscribed);

        let key = counter.get_key().to_owned();
        counter.unsubscribe().unwrap();

        awaitility::at_most(std::time::Duration::from_secs(1))
            .poll_interval(std::time::Duration::from_micros(100))
            .until(|| {
                counter.get_state() == DatatypeState::Disabled
                    && client.get_datatype(&key).is_none()
            });
    }

    #[test]
    #[instrument]
    fn can_unsubscribe_datatype_from_client_in_realtime() {
        let connectivity = LocalConnectivity::new_arc();
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        let counter = client
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        awaitility::at_most(std::time::Duration::from_secs(1))
            .poll_interval(std::time::Duration::from_micros(100))
            .until(|| counter.get_state() == DatatypeState::Subscribed);

        let removed = client.unsubscribe_datatype(counter.get_key()).unwrap();
        assert_eq!(removed.get_state(), DatatypeState::DueToUnsubscribe);
        assert!(client.get_datatype(counter.get_key()).is_some());

        awaitility::at_most(std::time::Duration::from_secs(1))
            .poll_interval(std::time::Duration::from_micros(100))
            .until(|| {
                removed.get_state() == DatatypeState::Disabled
                    && client.get_datatype(counter.get_key()).is_none()
            });
    }

    #[test]
    #[instrument]
    fn can_reject_unsubscribe_for_unmanaged_key() {
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build()
            .unwrap();

        assert!(matches!(
            client.unsubscribe_datatype("missing").err().unwrap(),
            crate::DatatypeError::Disallowed(_)
        ));
    }
}
