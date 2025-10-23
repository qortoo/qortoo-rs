use std::sync::Arc;

use parking_lot::RwLock;

use crate::{
    DataType, DatatypeBuilder, DatatypeState, IntoString,
    clients::{common::ClientCommon, datatype_manager::DatatypeManager},
    connectivity::{Connectivity, null_connectivity::NullConnectivity},
    datatypes::{datatype_set::DatatypeSet, option::DatatypeOption},
    errors::clients::ClientError,
};

/// A builder for constructing a [`Client`].
///
/// Use [`Client::builder`] to start, then call [`ClientBuilder::build`]
/// to obtain a ready-to-use client instance.
///
/// # Examples
/// ```
/// use syncyam::Client;
/// let client = Client::builder("doc-example", "ClientBuilder-test").build();
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
    pub fn build(self) -> Client {
        let common = ClientCommon::new_arc(
            self.collection.into_boxed_str(),
            self.alias.into_boxed_str(),
            self.connectivity,
        );
        Client {
            datatypes: RwLock::new(DatatypeManager::new(common.clone())),
            common,
        }
    }
}

/// Facade for creating and subscribing to SyncYam datatypes.
///
/// A `Client` is scoped by a logical `collection` and an `alias` that
/// are propagated into tracing metadata and used to associate created
/// datatypes with their owner.
///
/// Use [`Client::builder`] to construct a client, and the `create_datatype` / `subscribe_datatype` / `subscribe_or_create_datatype`
/// helpers to get specific datatypes.
pub struct Client {
    common: Arc<ClientCommon>,
    datatypes: RwLock<DatatypeManager>,
}

impl Client {
    /// Returns a ClientBuilder to construct a new client with
    /// the given `collection` and `alias`.
    ///
    /// # Examples
    /// ```
    /// use syncyam::Client;
    /// let client = Client::builder("col", "alias").build();
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
    ) -> Result<DatatypeSet, ClientError> {
        self.datatypes
            .write()
            .subscribe_or_create_datatype(&key, r#type, state, option)
    }

    /// Returns an existing datatype by `key`, if it has been created or
    /// subscribed via this client.
    pub fn get_datatype(&self, key: &str) -> Option<DatatypeSet> {
        self.datatypes.read().get_datatype(key)
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
    pub fn subscribe_datatype(&self, key: impl IntoString) -> DatatypeBuilder {
        DatatypeBuilder::new(self, key.into(), DatatypeState::DueToSubscribe)
    }

    /// Get `DatatypeBuilder` to create a `Datatype` identified by `key`.
    ///
    /// The `Datatype` built by this builder will be marked
    /// with [`DatatypeState::DueToCreate`].
    pub fn create_datatype(&self, key: impl IntoString) -> DatatypeBuilder {
        DatatypeBuilder::new(self, key.into(), DatatypeState::DueToCreate)
    }

    /// Get `DatatypeBuilder` to subscribe or create a `Datatype` identified by `key`.
    ///
    /// The `Datatype` built by this builder will be marked
    /// with [`DatatypeState::DueToSubscribeOrCreate`].
    pub fn subscribe_or_create_datatype(&self, key: impl IntoString) -> DatatypeBuilder {
        DatatypeBuilder::new(self, key.into(), DatatypeState::DueToSubscribeOrCreate)
    }
}

#[cfg(test)]
mod tests_client {
    use tracing::instrument;

    use crate::{
        Datatype, DatatypeState, clients::client::Client, utils::path::get_test_func_name,
    };

    #[test]
    fn can_assert_send_and_sync_traits() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Client>();
    }

    #[test]
    #[instrument]
    fn can_build_client() {
        let client = Client::builder("collection1", "alias1").build();
        assert_eq!(client.get_collection(), "collection1");
        assert_eq!(client.get_alias(), "alias1");
    }

    #[test]
    #[instrument]
    fn can_use_counter_from_client() {
        let client1 = Client::builder(module_path!(), get_test_func_name!()).build();

        assert!(client1.get_datatype("k1").is_none());

        let counter1 = client1.subscribe_datatype("k1").build_counter().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::DueToSubscribe);
        assert!(client1.get_datatype("k1").is_some());

        let client2 = Client::builder(module_path!(), module_path!()).build();
        let counter2 = client2.create_datatype("k1").build_counter().unwrap();
        assert_eq!(counter2.get_state(), DatatypeState::DueToCreate);

        let client3 = Client::builder(module_path!(), module_path!()).build();
        let counter3 = client3
            .subscribe_or_create_datatype("k1")
            .build_counter()
            .unwrap();
        assert_eq!(counter3.get_state(), DatatypeState::DueToSubscribeOrCreate);
    }
}
