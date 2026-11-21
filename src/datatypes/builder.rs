use std::collections::BTreeMap;

use dyn_fmt::AsStrFormatExt;

use crate::{
    Client, ClientError, Counter, DataType, DatatypeHandler, DatatypeState, Map, Variable,
    datatypes::{datatype_set::DatatypeSet, option::DatatypeOption},
    errors::{messages::client as msg, with_err_out},
    utils::name_validator::is_valid_datatype_key,
};

/// Configures and registers a datatype with a [`Client`].
///
/// The client method chooses the datatype's initial lifecycle state.
///
/// - [`Client::subscribe_datatype`] requests an existing datatype by key.
/// - [`Client::create_datatype`] requests a new datatype by key.
/// - [`Client::subscribe_or_create_datatype`] accepts either outcome.
///
/// Finish registration with [`build_counter`](Self::build_counter),
/// [`build_variable`](Self::build_variable), or [`build_map`](Self::build_map).
/// These methods validate the datatype key and register the handle locally.
/// The backend processes the request during synchronization.
/// A datatype key must be 1 to 255 bytes long, contain no NUL bytes, and
/// start with a character other than `$`.
/// See the [builder guide](https://github.com/qortoo/qortoo-rs/blob/main/docs/client-and-datatype-builder.md).
///
/// # Examples
///
/// ```
/// use qortoo::{
///     Client, DatatypeState, Datatype, LocalConnectivity,
///     SnapshotCodecKind, SnapshotCompression, SnapshotWriteOptions,
/// };
///
/// # #[cfg(feature = "snapshot-bitcode")]
/// # let codec = SnapshotCodecKind::Bitcode;
/// # #[cfg(not(feature = "snapshot-bitcode"))]
/// # let codec = SnapshotCodecKind::Postcard;
/// // Pause automatic synchronization to inspect the initial builder states.
/// let connectivity = LocalConnectivity::new_arc_with_snapshot_options(SnapshotWriteOptions {
///     codec,
///     compression: SnapshotCompression::None,
/// });
/// connectivity.set_realtime(false);
/// let client = Client::builder("docs-example", "DatatypeBuilder-test")
///     .with_connectivity(connectivity)
///     .build_and_open()
///     .unwrap();
/// assert_eq!(
///     client.subscribe_datatype("k1").build_counter().unwrap().get_state(),
///     DatatypeState::Subscribing
/// );
/// assert_eq!(
///     client.create_datatype("k2").build_variable().unwrap().get_state(),
///     DatatypeState::Creating
/// );
/// assert_eq!(
///     client
///         .subscribe_or_create_datatype("k3")
///         .build_map()
///         .unwrap()
///         .get_state(),
///     DatatypeState::SubscribingOrCreating
/// );
/// ```
pub struct DatatypeBuilder<'c> {
    client: &'c Client,
    key: String,
    state: DatatypeState,
    option: DatatypeOption,
    is_readonly: bool,
    handlers: BTreeMap<usize, DatatypeHandler>,
}

impl<'c> DatatypeBuilder<'c> {
    /// Creates a new builder. This is used internally by [`Client`].
    pub(crate) fn new(client: &'c Client, key: String, state: DatatypeState) -> Self {
        Self {
            client,
            key,
            state,
            option: DatatypeOption::default(),
            is_readonly: false,
            handlers: Default::default(),
        }
    }

    /// Registers a [`Counter`] with the builder's client.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::FailedToSubscribeOrCreateDatatype`] if the key
    /// is invalid or the client already manages that key, or
    /// [`ClientError::ClientNotOpen`] if the client has not been opened.
    pub fn build_counter(self) -> Result<Counter, ClientError> {
        Ok(self
            .build(DataType::Counter)?
            .into_counter()
            .expect("DatatypeSet must match the requested type"))
    }

    /// Registers a [`Variable`] with the builder's client.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::FailedToSubscribeOrCreateDatatype`] if the key
    /// is invalid or the client already manages that key, or
    /// [`ClientError::ClientNotOpen`] if the client has not been opened.
    pub fn build_variable(self) -> Result<Variable, ClientError> {
        Ok(self
            .build(DataType::Variable)?
            .into_variable()
            .expect("DatatypeSet must match the requested type"))
    }

    /// Registers a [`Map`] with the builder's client.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::FailedToSubscribeOrCreateDatatype`] if the key
    /// is invalid or the client already manages that key, or
    /// [`ClientError::ClientNotOpen`] if the client has not been opened.
    pub fn build_map(self) -> Result<Map, ClientError> {
        Ok(self
            .build(DataType::Map)?
            .into_map()
            .expect("DatatypeSet must match the requested type"))
    }

    fn build(self, datatype: DataType) -> Result<DatatypeSet, ClientError> {
        if !is_valid_datatype_key(&self.key) {
            return Err(with_err_out!(
                ClientError::FailedToSubscribeOrCreateDatatype(
                    msg::INVALID_DATATYPE_KEY_TEMPLATE.format(&[self.key])
                )
            ));
        }
        self.client.do_subscribe_or_create_datatype(
            self.key,
            datatype,
            self.state,
            self.option,
            self.is_readonly,
            self.handlers,
        )
    }

    /// Sets the byte limit for the push buffer.
    ///
    /// The buffer holds local transactions pending synchronization. The default
    /// limit is 100 MB and `size` is clamped to 1 MB through 1 GB. A transaction
    /// that would exceed the limit fails to commit and is rolled back.
    ///
    /// # Examples
    ///
    /// ```
    /// use qortoo::Client;
    /// let client = Client::builder("doc-example", "push-buffer-test")
    ///     .build_and_open()
    ///     .unwrap();
    /// let counter = client
    ///     .create_datatype("my-counter")
    ///     .with_max_memory_size_of_push_buffer(20_000_000) // 20MB
    ///     .build_counter()
    ///     .unwrap();
    /// ```
    pub fn with_max_memory_size_of_push_buffer(mut self, size: u64) -> Self {
        let option = DatatypeOption::new(size);
        self.option = option;
        self
    }

    /// Configures the datatype to reject local writes.
    ///
    /// The read-only setting also applies after a subscription is confirmed.
    /// Read-only handles can read local state and receive remote updates.
    ///
    /// # Examples
    ///
    /// ```
    /// use qortoo::Client;
    /// let client = Client::builder("doc-example", "readonly-test")
    ///     .build_and_open()
    ///     .unwrap();
    /// let counter = client
    ///     .subscribe_datatype("read-only-counter")
    ///     .with_readonly()
    ///     .build_counter()
    ///     .unwrap();
    /// assert_eq!(counter.get_value(), 0);
    /// assert!(counter.increase().is_err());
    /// ```
    pub fn with_readonly(mut self) -> Self {
        self.is_readonly = true;
        self
    }

    /// Registers a [`DatatypeHandler`] at the given `priority`.
    ///
    /// Handlers run in ascending priority order. Setting a priority again
    /// replaces the handler previously configured at that priority.
    ///
    /// # Examples
    ///
    /// ```
    /// use qortoo::{Client, DatatypeHandler};
    /// let client = Client::builder("doc-example", "with-handler-test")
    ///     .build_and_open()
    ///     .unwrap();
    /// let counter = client
    ///     .create_datatype("my-counter")
    ///     .with_handler(0, DatatypeHandler::new())
    ///     .build_counter()
    ///     .unwrap();
    /// ```
    pub fn with_handler(mut self, priority: usize, handler: DatatypeHandler) -> Self {
        self.handlers.insert(priority, handler);
        self
    }
}

#[cfg(test)]
mod tests_datatype_builder {
    use rstest::rstest;
    use tracing::instrument;

    use crate::{
        Client, ClientError, Datatype, DatatypeError, DatatypeState,
        utils::test_utils::{get_test_collection_name, get_test_func_name, local_connectivity},
    };

    #[test]
    #[instrument]
    fn can_show_how_to_use_datatype_builder() {
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build_and_open()
            .unwrap();
        let _counter = client
            .subscribe_datatype(get_test_func_name!())
            .with_max_memory_size_of_push_buffer(20_000_000)
            .build_counter()
            .unwrap();
    }

    #[test]
    #[instrument]
    fn can_create_readonly_counter() {
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build_and_open()
            .unwrap();
        let counter = client
            .subscribe_datatype(get_test_func_name!())
            .with_readonly()
            .build_counter()
            .unwrap();

        // Read operations should work
        assert_eq!(counter.get_value(), 0);

        // Write operations should fail
        assert!(matches!(
            counter.increase().unwrap_err(),
            DatatypeError::NotWritable(_)
        ));

        // Transaction should fail — either NotWritable (state not yet synced) or
        // ReadonlyViolation (state already synced to Subscribed by the time this runs)
        let tx_result = counter.transaction("test-tx", |c| {
            c.increase().unwrap();
            Ok(())
        });
        assert!(matches!(
            tx_result.unwrap_err(),
            DatatypeError::NotWritable(_) | DatatypeError::ReadonlyViolation
        ));
        assert_eq!(counter.get_value(), 0);
    }

    #[test]
    #[instrument]
    fn can_check_read_only_state() {
        // Inspect builder states before synchronization can advance them.
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build_and_open()
            .unwrap();

        let counter = client.create_datatype("create_dt").build_counter().unwrap();

        assert!(counter.increase().is_ok());

        let counter = client
            .subscribe_datatype("subscribe_dt")
            .build_counter()
            .unwrap();
        assert_eq!(counter.get_state(), DatatypeState::Subscribing);
        assert!(matches!(
            counter.increase().unwrap_err(),
            DatatypeError::NotWritable(_)
        ));

        let counter = client
            .subscribe_or_create_datatype("subscribe_or_create_dt")
            .build_counter()
            .unwrap();
        assert_eq!(counter.get_state(), DatatypeState::SubscribingOrCreating);
        assert!(counter.increase().is_ok());
    }

    #[rstest]
    #[case::success("hello", true)]
    #[case::fail("", false)]
    #[case::fail("hello\0world", false)]
    #[instrument]
    fn can_fail_with_invalid_datatype_keys(#[case] key: &str, #[case] success_expected: bool) {
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build_and_open()
            .unwrap();
        let ret = client.create_datatype(key).build_counter();
        assert_eq!(ret.is_ok(), success_expected);
        if !success_expected {
            let error = ret.err().unwrap();
            assert_eq!(
                error,
                ClientError::FailedToSubscribeOrCreateDatatype("".to_string())
            );
            assert_eq!(
                error.to_string(),
                format!(
                    "[E101] cannot subscribe or create datatype: invalid datatype key '{key}': must be 1-255 bytes, must not contain null characters (\\0), and must not start with '$'"
                )
            );
        }
    }

    #[test]
    #[instrument]
    fn can_build_a_variable_via_datatype_builder() {
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build_and_open()
            .unwrap();
        let variable = client
            .create_datatype(get_test_func_name!())
            .build_variable()
            .unwrap();
        assert_eq!(variable.get_type(), crate::DataType::Variable);
        assert_eq!(
            variable.get::<serde_json::Value>().unwrap(),
            serde_json::Value::Null
        );
    }
}
