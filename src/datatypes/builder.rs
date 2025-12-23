use crate::{
    Client, ClientError, Counter, DataType, DatatypeState,
    datatypes::{datatype_set::DatatypeSet, option::DatatypeOption},
};

/// A builder for constructing Qortoo datatypes with configurable options.
///
/// `DatatypeBuilder` is obtained from a [`Client`] via one of:
/// - [`Client::subscribe_datatype`] — subscribe to an existing datatype by key
/// - [`Client::create_datatype`] — create a new datatype by key
/// - [`Client::subscribe_or_create_datatype`] — subscribe if exists, otherwise create
///
/// construction by calling a concrete builder, such as [`DatatypeBuilder::build_counter`].
///
/// The provided values are clamped to an allowed range. If a value is smaller
/// than the minimum, the minimum is used; if larger than the maximum, the
/// maximum is used.
///
/// The builder preserves the intended lifecycle state based on how it was
/// obtained from [`Client`]. For example:
/// ```
/// use qortoo::{Client, DatatypeState, Datatype};
/// let client = Client::builder("docs-example", "DatatypeBuilder-test").build();
/// assert_eq!(
///     client.subscribe_datatype("k1").build_counter().unwrap().get_state(),
///     DatatypeState::DueToSubscribe
/// );
/// assert_eq!(
///     client.create_datatype("k2").build_counter().unwrap().get_state(),
///     DatatypeState::DueToCreate
/// );
/// assert_eq!(
///     client
///         .subscribe_or_create_datatype("k3")
///         .build_counter()
///         .unwrap()
///         .get_state(),
///     DatatypeState::DueToSubscribeOrCreate
/// );
/// ```
pub struct DatatypeBuilder<'c> {
    client: &'c Client,
    key: String,
    state: DatatypeState,
    option: DatatypeOption,
    is_readonly: bool,
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
        }
    }

    /// Finalizes the builder and constructs a [`Counter`].
    ///
    /// Uses the builder's lifecycle state (subscribe/create/subscribe-or-create)
    /// to return a ready-to-use counter.
    ///
    /// # Errors
    /// Returns [`ClientError`] if the underlying creation/subscription fails.
    ///
    /// # Examples
    /// ```
    /// use qortoo::Client;
    /// let client = Client::builder("doc-example", "build_counter-test").build();
    /// let counter = client
    ///     .create_datatype("counter-1")
    ///     .build_counter()
    ///     .unwrap();
    /// assert_eq!(counter.get_value(), 0);
    /// ```
    pub fn build_counter(self) -> Result<Counter, ClientError> {
        let ds = self.client.do_subscribe_or_create_datatype(
            self.key,
            DataType::Counter,
            self.state,
            self.option,
            self.is_readonly,
        )?;
        let DatatypeSet::Counter(c) = ds;
        Ok(c)
    }

    /// Configures the maximum memory size for the push buffer.
    ///
    /// The push buffer stores pending operations before they are synchronized.
    /// When the buffer exceeds this limit, further write operations will fail
    /// until pending operations are synchronized.
    ///
    /// # Arguments
    ///
    /// * `size` - Maximum memory size in bytes (will be clamped to allowed range)
    ///
    /// # Examples
    ///
    /// ```
    /// use qortoo::Client;
    /// let client = Client::builder("doc-example", "push-buffer-test").build();
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

    /// Marks this datatype as read-only.
    ///
    /// Read-only datatypes reject all write operations, making them
    /// suitable for scenarios where you want to observe state without
    /// modification.
    ///
    /// # Examples
    ///
    /// ```
    /// use qortoo::Client;
    /// let client = Client::builder("doc-example", "readonly-test").build();
    /// let counter = client
    ///     .subscribe_datatype("read-only-counter")
    ///     .with_readonly()
    ///     .build_counter()
    ///     .unwrap();
    /// // Write operations will fail on readonly counters
    /// ```
    pub fn with_readonly(mut self) -> Self {
        self.is_readonly = true;
        self
    }
}

#[cfg(test)]
mod tests_datatype_builder {
    use tracing::instrument;

    use crate::{Client, Datatype, DatatypeError, DatatypeState, utils::path::get_test_func_name};

    #[test]
    #[instrument]
    fn can_show_how_to_use_datatype_builder() {
        let client = Client::builder(module_path!(), get_test_func_name!()).build();
        let _counter = client
            .subscribe_datatype(get_test_func_name!())
            .with_max_memory_size_of_push_buffer(20_000_000)
            .build_counter()
            .unwrap();
    }

    #[test]
    #[instrument]
    fn can_create_readonly_counter() {
        let client = Client::builder(module_path!(), get_test_func_name!()).build();
        let counter = client
            .subscribe_datatype(get_test_func_name!())
            .with_readonly()
            .build_counter()
            .unwrap();

        // Read operations should work
        assert_eq!(counter.get_value(), 0);

        // Write operations should fail
        assert_eq!(
            counter.increase().unwrap_err(),
            DatatypeError::FailedToWrite("".into())
        );

        // Transaction should fail
        let tx_result = counter.transaction("test-tx", |c| {
            c.increase().unwrap();
            Ok(())
        });
        assert_eq!(
            tx_result.unwrap_err(),
            DatatypeError::FailedToWrite("".into())
        );
        assert_eq!(counter.get_value(), 0);
    }

    #[test]
    #[instrument]
    fn can_check_read_only_state() {
        let client = Client::builder(module_path!(), get_test_func_name!()).build();

        let counter = client.create_datatype("create_dt").build_counter().unwrap();
        assert_eq!(counter.get_state(), DatatypeState::DueToCreate);
        assert!(counter.increase().is_ok());

        let counter = client
            .subscribe_datatype("subscribe_dt")
            .build_counter()
            .unwrap();
        assert_eq!(counter.get_state(), DatatypeState::DueToSubscribe);
        assert_eq!(
            counter.increase().unwrap_err(),
            DatatypeError::FailedToWrite("".into())
        );

        let counter = client
            .subscribe_or_create_datatype("subscribe_or_create_dt")
            .build_counter()
            .unwrap();
        assert_eq!(counter.get_state(), DatatypeState::DueToSubscribeOrCreate);
        assert!(counter.increase().is_ok());
    }
}
