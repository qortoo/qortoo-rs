use crate::{
    Client, ClientError, Counter, DataType, DatatypeSet, DatatypeState,
    datatypes::option::DatatypeOption, defaults,
};

/// A builder for constructing SyncYam datatypes with configurable options.
///
/// `DatatypeBuilder` is obtained from a [`Client`] via one of:
/// - [`Client::subscribe_datatype`] — subscribe to an existing datatype by key
/// - [`Client::create_datatype`] — create a new datatype by key
/// - [`Client::subscribe_or_create_datatype`] — subscribe if exists, otherwise create
///
/// Once created, you can tune rollback-related options and then finalize
/// construction by calling a concrete builder, such as [`DatatypeBuilder::build_counter`].
///
/// Rollback-related options control how much history/memory is kept to support
/// transactional rollbacks:
/// - max number of rollback transactions
/// - max size of rollback memory (bytes)
///
/// The provided values are clamped to an allowed range. If a value is smaller
/// than the minimum, the minimum is used; if larger than the maximum, the
/// maximum is used.
///
/// # Examples
/// Create a counter with custom rollback limits:
/// ```
/// use syncyam::Client;
/// let client = Client::builder("docs-collection", "docs-app").build();
/// let counter = client
///     .create_datatype("docs-counter")
///     .with_max_num_of_rollback_transactions(1_000)
///     .with_max_size_of_rollback_memory(1_000_000)
///     .build_counter()
///     .unwrap();
/// assert_eq!(counter.get_value(), 0);
/// ```
///
/// The builder preserves the intended lifecycle state based on how it was
/// obtained from [`Client`]. For example:
/// ```
/// use syncyam::{Client, DatatypeState, Datatype};
/// let client = Client::builder("docs-collection", "docs-app").build();
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
    max_num_of_rollback_transactions: u32,
    max_size_of_rollback_memory: u64,
}

impl<'c> DatatypeBuilder<'c> {
    /// Creates a new builder. This is used internally by [`Client`].
    pub(crate) fn new(client: &'c Client, key: String, state: DatatypeState) -> Self {
        Self {
            client,
            key,
            state,
            max_num_of_rollback_transactions: defaults::DEFAULT_MAX_NUM_OF_ROLLBACK_TRANSACTIONS,
            max_size_of_rollback_memory: defaults::DEFAULT_MAX_SIZE_OF_ROLLBACK_MEMORY,
        }
    }

    /// Sets the maximum number of rollback transactions to retain.
    ///
    /// The value is clamped to a valid range. If `max` is smaller than the
    /// lower bound, the lower bound is used. If it exceeds the upper bound,
    /// the upper bound is used.
    ///
    /// Returns the builder for method chaining.
    pub fn with_max_num_of_rollback_transactions(mut self, max: u32) -> Self {
        self.max_num_of_rollback_transactions = max.clamp(
            defaults::LOWER_MAX_NUM_OF_ROLLBACK_TRANSACTIONS,
            defaults::UPPER_MAX_NUM_OF_ROLLBACK_TRANSACTIONS,
        );
        self
    }

    /// Sets the maximum size of rollback memory (in bytes).
    ///
    /// The value is clamped to a valid range. If `max` is smaller than the
    /// lower bound, the lower bound is used. If it exceeds the upper bound,
    /// the upper bound is used.
    ///
    /// Returns the builder for method chaining.
    pub fn with_max_size_of_rollback_memory(mut self, max: u64) -> Self {
        self.max_size_of_rollback_memory = max.clamp(
            defaults::LOWER_MAX_SIZE_OF_ROLLBACK_MEMORY,
            defaults::UPPER_MAX_SIZE_OF_ROLLBACK_MEMORY,
        );
        self
    }

    /// Finalizes the builder and constructs a [`Counter`].
    ///
    /// Applies the configured rollback options and uses the builder's
    /// lifecycle state (subscribe/create/subscribe-or-create) to return
    /// a ready-to-use counter.
    ///
    /// # Errors
    /// Returns [`ClientError`] if the underlying creation/subscription fails.
    ///
    /// # Examples
    /// ```
    /// use syncyam::Client;
    /// let client = Client::builder("col", "alias").build();
    /// let counter = client
    ///     .create_datatype("counter-1")
    ///     .with_max_num_of_rollback_transactions(500)
    ///     .with_max_size_of_rollback_memory(256 * 1024)
    ///     .build_counter()
    ///     .unwrap();
    /// assert_eq!(counter.get_value(), 0);
    /// ```
    pub fn build_counter(self) -> Result<Counter, ClientError> {
        let option = DatatypeOption::new(
            self.max_num_of_rollback_transactions,
            self.max_size_of_rollback_memory,
        );
        match self.client.do_subscribe_or_create_datatype(
            self.key,
            DataType::Counter,
            self.state,
            option,
        ) {
            Ok(ds) => match ds {
                DatatypeSet::Counter(counter) => Ok(counter),
            },
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests_datatype_builder {
    use crate::{Client, defaults};

    #[test]
    fn can_use_rollback_related_datatype_builder() {
        let client = Client::builder(module_path!(), module_path!()).build();
        let mut builder = client.subscribe_datatype(module_path!());
        assert_eq!(
            defaults::DEFAULT_MAX_SIZE_OF_ROLLBACK_MEMORY,
            builder.max_size_of_rollback_memory
        );
        assert_eq!(
            defaults::DEFAULT_MAX_NUM_OF_ROLLBACK_TRANSACTIONS,
            builder.max_num_of_rollback_transactions
        );

        builder = builder
            .with_max_size_of_rollback_memory(0)
            .with_max_num_of_rollback_transactions(0);
        assert_eq!(
            defaults::LOWER_MAX_SIZE_OF_ROLLBACK_MEMORY,
            builder.max_size_of_rollback_memory
        );
        assert_eq!(
            defaults::LOWER_MAX_NUM_OF_ROLLBACK_TRANSACTIONS,
            builder.max_num_of_rollback_transactions
        );

        builder = builder
            .with_max_size_of_rollback_memory(defaults::UPPER_MAX_SIZE_OF_ROLLBACK_MEMORY + 1)
            .with_max_num_of_rollback_transactions(
                defaults::UPPER_MAX_NUM_OF_ROLLBACK_TRANSACTIONS + 1,
            );
        assert_eq!(
            defaults::UPPER_MAX_SIZE_OF_ROLLBACK_MEMORY,
            builder.max_size_of_rollback_memory
        );
        assert_eq!(
            defaults::UPPER_MAX_NUM_OF_ROLLBACK_TRANSACTIONS,
            builder.max_num_of_rollback_transactions
        );

        builder = builder
            .with_max_size_of_rollback_memory(defaults::LOWER_MAX_SIZE_OF_ROLLBACK_MEMORY + 1)
            .with_max_num_of_rollback_transactions(
                defaults::LOWER_MAX_NUM_OF_ROLLBACK_TRANSACTIONS + 1,
            );
        assert_eq!(
            defaults::LOWER_MAX_SIZE_OF_ROLLBACK_MEMORY + 1,
            builder.max_size_of_rollback_memory
        );
        assert_eq!(
            defaults::LOWER_MAX_NUM_OF_ROLLBACK_TRANSACTIONS + 1,
            builder.max_num_of_rollback_transactions
        );

        let _counter = builder.build_counter().unwrap();
    }
}
