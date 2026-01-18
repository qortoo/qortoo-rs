use std::sync::Arc;

use dyn_fmt::AsStrFormatExt;
use parking_lot::RwLock;

use crate::{
    DataType, DatatypeBuilder, DatatypeState, IntoString,
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
        Ok(Client {
            datatypes: RwLock::new(DatatypeManager::new(common.clone())),
            common,
        })
    }

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
    datatypes: RwLock<DatatypeManager>,
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
    ) -> Result<DatatypeSet, ClientError> {
        self.datatypes.write().subscribe_or_create_datatype(
            &key,
            r#type,
            state,
            option,
            is_readonly,
        )
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
        Datatype, DatatypeState,
        clients::client::Client,
        utils::path::{get_test_collection_name, get_test_func_name},
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
        let client1 = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build()
            .unwrap();

        assert!(client1.get_datatype("k1").is_none());

        let counter1 = client1.subscribe_datatype("k1").build_counter().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::DueToSubscribe);
        assert!(client1.get_datatype("k1").is_some());

        let client2 = Client::builder(get_test_collection_name!(), get_test_collection_name!())
            .build()
            .unwrap();
        let counter2 = client2.create_datatype("k1").build_counter().unwrap();
        assert_eq!(counter2.get_state(), DatatypeState::DueToCreate);

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
}
