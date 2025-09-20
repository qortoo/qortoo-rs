use crate::{
    Client, ClientError, Counter, DataType, DatatypeSet, DatatypeState,
    datatypes::option::DatatypeOption,
};

/// A builder for constructing SyncYam datatypes with configurable options.
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
}

impl<'c> DatatypeBuilder<'c> {
    /// Creates a new builder. This is used internally by [`Client`].
    pub(crate) fn new(client: &'c Client, key: String, state: DatatypeState) -> Self {
        Self { client, key, state }
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
    /// use syncyam::Client;
    /// let client = Client::builder("col", "alias").build();
    /// let counter = client
    ///     .create_datatype("counter-1")
    ///     .build_counter()
    ///     .unwrap();
    /// assert_eq!(counter.get_value(), 0);
    /// ```
    pub fn build_counter(self) -> Result<Counter, ClientError> {
        let option = DatatypeOption::new();
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
    use crate::Client;

    #[test]
    fn can_use_rollback_related_datatype_builder() {
        let client = Client::builder(module_path!(), module_path!()).build();
        let builder = client.subscribe_datatype(module_path!());

        let _counter = builder.build_counter().unwrap();
    }
}
