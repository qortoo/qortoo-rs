use std::sync::Arc;

use serde::{Serialize, de::DeserializeOwned};
use tracing::trace;

use crate::{
    DatatypeError, IntoString,
    datatypes::{
        common::{ReturnType, datatype_instrument},
        datatype::DatatypeBlanket,
        transactional::{TransactionContext, TransactionalDatatype},
    },
    errors::{BoxedError, datatypes::InternalReason, messages::datatype as msg},
    operations::Operation,
    utils::json::{decode_json_value, encode_json_value, validate_json_str},
};

/// A conflict-free datatype that stores one JSON value.
///
/// The value starts as JSON `null`. Writes converge to the value with the
/// greatest logical timestamp.
///
/// Use [`DatatypeBuilder::build_variable`](crate::DatatypeBuilder::build_variable)
/// to obtain a handle and [`Datatype`](crate::Datatype) for lifecycle,
/// synchronization, and handlers. Reads are local and available in every
/// lifecycle state, including on read-only handles. See the
/// [Variable guide](https://github.com/qortoo/qortoo-rs/blob/main/docs/variable.md).
///
/// # Examples
///
/// ```
/// use qortoo::Client;
/// use serde_json::{Value, json};
///
/// let client = Client::builder("doc-example", "variable").build().unwrap();
/// let variable = client.create_datatype("status").build_variable().unwrap();
///
/// assert_eq!(variable.get::<Option<String>>().unwrap(), None);
/// assert_eq!(variable.set("ready").unwrap(), Value::Null);
/// assert_eq!(variable.get::<String>().unwrap(), "ready");
/// assert_eq!(variable.set(&true).unwrap(), json!("ready"));
/// assert!(variable.get::<bool>().unwrap());
/// ```
#[derive(Clone)]
pub struct Variable {
    datatype: Arc<TransactionalDatatype>,
    tx_ctx: Arc<TransactionContext>,
}

impl Variable {
    pub(crate) fn new(datatype: Arc<TransactionalDatatype>) -> Self {
        Variable {
            datatype,
            tx_ctx: Default::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(state: crate::DatatypeState) -> Self {
        Self::new(TransactionalDatatype::new_arc(
            crate::datatypes::common::new_attribute!(crate::DataType::Variable),
            state,
            Default::default(),
        ))
    }

    datatype_instrument! {
    /// Stores a serializable value as JSON.
    ///
    /// Returns the previous value as a [`serde_json::Value`], which is JSON
    /// `null` when replacing the initial value. Each write may store a different JSON type.
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError::ValueConversion`] if encoding the new value
    /// fails or if the previous value cannot be decoded as a
    /// [`serde_json::Value`]. Encoding failure leaves the value unchanged.
    /// Decoding the previous value happens after the write, so the new value
    /// may already be stored when this method returns the error. Use
    /// [`set_raw`](Self::set_raw) to replace a value while returning its original bytes.
    /// Also returns an error if the write is rejected or cannot commit locally.
    ///
    /// Inside [`transaction`](Self::transaction), return the error from the
    /// callback to roll back the batch.
    pub fn set<T: Serialize + ?Sized>(
        &self,
        value: &T,
    ) -> Result<serde_json::Value, DatatypeError> {
        let previous = self.set_encoded(encode_json_value(value)?)?;
        decode_json_value(&previous)
    }}

    datatype_instrument! {
    /// Stores exactly one UTF-8 JSON value, preserving the supplied bytes verbatim.
    ///
    /// Returns the previous value as its original bytes, which are `b"null"`
    /// when replacing the initial value. Whitespace, property order, and numeric
    /// formatting are preserved.
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError::ValueConversion`] for invalid input before
    /// changing the value. Also returns an error if the write is rejected or
    /// cannot commit locally.
    pub fn set_raw(&self, json: &[u8]) -> Result<Box<[u8]>, DatatypeError> {
        let previous = self.set_encoded(validate_json_str(json)?.as_bytes().into())?;
        Ok(Box::from(&*previous))
    }}

    /// Shared write path: records `encoded` as an LWW set and returns the bytes the
    /// variable held just before it.
    fn set_encoded(&self, encoded: Box<[u8]>) -> Result<Arc<[u8]>, DatatypeError> {
        let op = Operation::new_variable_set(encoded);
        let ret = self
            .datatype
            .execute_local_operation_as_tx(self.tx_ctx.clone(), op)?;
        trace!("set -> {ret:?}");
        match ret {
            ReturnType::Variable(previous) => Ok(previous),
            _ => Err(
                InternalReason::ExecuteOperation(msg::UNEXPECTED_RETURN_TYPE.into()).into_error(),
            ),
        }
    }

    /// Returns the current value decoded as `T`.
    ///
    /// The initial value and an explicitly stored JSON `null` both decode as
    /// `None` for `Option<U>` or as [`serde_json::Value::Null`].
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError::ValueConversion`] if the stored value cannot be
    /// decoded as `T`.
    pub fn get<T: DeserializeOwned>(&self) -> Result<T, DatatypeError> {
        decode_json_value(&self.shared_value())
    }

    /// Returns a copy of the stored JSON bytes.
    ///
    /// The initial value is `b"null"`. An explicitly stored value is returned
    /// exactly as supplied by [`set_raw`](Self::set_raw) or encoded by [`set`](Self::set).
    pub fn get_raw(&self) -> Box<[u8]> {
        Box::from(&*self.shared_value())
    }

    /// Clones the stored JSON bytes out from under the datatype read lock.
    fn shared_value(&self) -> Arc<[u8]> {
        self.datatype
            .core
            .mutable
            .read()
            .crdt
            .as_variable()
            .expect("variable datatype must contain a variable crdt")
            .shared_value()
    }

    datatype_instrument! {
    /// Executes a batch of writes as one local transaction.
    ///
    /// Use the callback handle or its clones for writes and finish that work
    /// before returning. Reads may observe uncommitted writes. `tag` labels the
    /// transaction. On success, the writes have committed locally. See
    /// [transaction guarantees](https://github.com/qortoo/qortoo-rs/blob/main/docs/transaction-and-rollback.md#rules-and-guarantees).
    ///
    /// # Errors
    ///
    /// Returns an error if writes are disallowed or the batch cannot commit
    /// locally. A callback error becomes [`DatatypeError::TransactionFailed`].
    /// Callback and commit failures roll back the batch. On the callback thread,
    /// starting another transaction on the same datatype or writing through a
    /// captured original handle returns [`DatatypeError::Disallowed`].
    ///
    /// # Examples
    ///
    /// The raw setter accepts JSON numbers such as `1e400` that
    /// [`serde_json::Value`] cannot represent. Returning the conversion error
    /// from the callback rolls back the write.
    ///
    /// ```
    /// use qortoo::{Client, DatatypeError};
    ///
    /// let client = Client::builder("doc-example", "variable-transaction").build().unwrap();
    /// let variable = client.create_datatype("measurement").build_variable().unwrap();
    /// variable.set_raw(b"1e400").unwrap();
    ///
    /// let result = variable.transaction("replace measurement", |measurement| {
    ///     measurement.set(&7)?;
    ///     Ok(())
    /// });
    /// assert!(matches!(result, Err(DatatypeError::TransactionFailed(_))));
    /// assert_eq!(&*variable.get_raw(), b"1e400");
    /// ```
    pub fn transaction<T>(
        &self,
        tag: impl IntoString,
        tx_func: T,
    ) -> Result<(), DatatypeError>
    where
        T: FnOnce(Self) -> Result<(), BoxedError> + Send + Sync + 'static,
    {
        self.datatype.check_writable()?;
        let this_tx_ctx = Arc::new(TransactionContext::new(tag));
        let this_tx_ctx_clone = this_tx_ctx.clone();
        let do_tx_func = move || {
            let mut variable_clone = self.clone();
            variable_clone.tx_ctx = this_tx_ctx_clone.clone();
            match tx_func(variable_clone) {
                Ok(_) => Ok(()),
                Err(e) => Err(DatatypeError::TransactionFailed(e.to_string())),
            }
        };
        self.datatype.do_transaction(this_tx_ctx, do_tx_func)
    }}
}

impl DatatypeBlanket for Variable {
    fn get_core(&self) -> &TransactionalDatatype {
        self.datatype.as_ref()
    }
}

#[cfg(test)]
mod tests_variable {
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};
    use tracing::instrument;

    use crate::{
        Client, DataType, DatatypeError, DatatypeState,
        datatypes::{datatype::Datatype, variable::Variable},
        utils::test_utils::{
            get_test_collection_name, get_test_func_name, get_test_ids, local_connectivity,
        },
    };

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Profile {
        name: String,
        age: u32,
    }

    fn sample_profile() -> Profile {
        Profile {
            name: "qortoo".to_string(),
            age: 3,
        }
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum Status {
        Active,
        Retired { reason: String },
    }

    #[test]
    #[instrument]
    fn can_round_trip_typed_values() {
        let variable = Variable::new_for_test(DatatypeState::Creating);

        assert_eq!(variable.set(&sample_profile()).unwrap(), Value::Null);
        assert_eq!(variable.get::<Profile>().unwrap(), sample_profile());
        assert_eq!(
            variable.get::<Option<Profile>>().unwrap(),
            Some(sample_profile())
        );

        let retired = Status::Retired {
            reason: "eol".to_string(),
        };
        variable.set(&retired).unwrap();
        assert_eq!(variable.get::<Status>().unwrap(), retired);
        variable.set(&Status::Active).unwrap();
        assert_eq!(variable.get::<Status>().unwrap(), Status::Active);
    }

    #[test]
    #[instrument]
    fn can_reject_a_shape_mismatched_get() {
        let variable = Variable::new_for_test(DatatypeState::Creating);
        variable.set("not a number").unwrap();

        let error = variable.get::<i64>().unwrap_err();
        assert_eq!(error, DatatypeError::ValueConversion(String::new()));
        // The failed Get did not change the stored value.
        assert_eq!(variable.get::<String>().unwrap(), "not a number");
    }

    #[test]
    #[instrument]
    fn can_return_the_previous_value_across_json_types() {
        let variable = Variable::new_for_test(DatatypeState::Creating);

        assert_eq!(variable.set(&42_i64).unwrap(), Value::Null);
        let previous = variable.set("text").unwrap();
        assert_eq!(previous, json!(42));
        let previous = variable.set(&vec![true, false]).unwrap();
        assert_eq!(previous, json!("text"));
        assert_eq!(variable.get::<Vec<bool>>().unwrap(), vec![true, false]);
    }

    #[test]
    #[instrument]
    fn can_read_initial_and_explicit_null_into_nullable_destinations() {
        let variable = Variable::new_for_test(DatatypeState::Creating);
        assert_eq!(variable.get::<Option<i64>>().unwrap(), None);
        assert_eq!(variable.get::<Value>().unwrap(), Value::Null);

        variable.set(&1_i64).unwrap();
        assert_eq!(variable.set(&Value::Null).unwrap(), json!(1));
        assert_eq!(variable.get::<Option<i64>>().unwrap(), None);
    }

    #[test]
    #[instrument]
    fn can_reject_null_for_a_non_nullable_destination() {
        let variable = Variable::new_for_test(DatatypeState::Creating);
        let error = variable.get::<i64>().unwrap_err();
        assert_eq!(error, DatatypeError::ValueConversion(String::new()));
    }

    #[test]
    #[instrument]
    fn can_keep_the_variable_unchanged_when_encoding_fails() {
        let variable = Variable::new_for_test(DatatypeState::Creating);
        let map = std::collections::BTreeMap::from([((1, 2), "x")]);
        let version = variable.get_client_version();

        let error = variable.set(&map).unwrap_err();
        assert_eq!(error, DatatypeError::ValueConversion(String::new()));
        assert_eq!(variable.get_client_version(), version);
        assert_eq!(variable.get::<Value>().unwrap(), Value::Null);
        // The failed set produced no operation: the next set still sees the initial null.
        assert_eq!(variable.set(&1_i64).unwrap(), Value::Null);
    }

    #[test]
    #[instrument]
    fn can_read_without_changing_state_version_or_push_buffer() {
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let (collection, key, _) = get_test_ids!();
        let client = Client::builder(collection, "read-invariants")
            .with_connectivity(connectivity)
            .build()
            .unwrap();
        let variable = client.create_datatype(key).build_variable().unwrap();
        variable.set(&sample_profile()).unwrap();
        let state = variable.get_state();
        let version = variable.get_client_version();
        let buffered_transactions = variable
            .datatype
            .core
            .mutable
            .read()
            .push_buffer
            .iter()
            .count();

        assert_eq!(variable.get::<Profile>().unwrap(), sample_profile());
        assert_eq!(variable.get_state(), state);
        assert_eq!(variable.get_client_version(), version);
        assert_eq!(
            variable
                .datatype
                .core
                .mutable
                .read()
                .push_buffer
                .iter()
                .count(),
            buffered_transactions
        );
    }

    #[test]
    #[instrument]
    fn can_reject_a_set_when_the_state_is_not_writable() {
        let variable = Variable::new_for_test(DatatypeState::Disabled);
        let error = variable.set(&1_i64).unwrap_err();
        assert_eq!(error, DatatypeError::NotWritable(String::new()));
    }

    #[test]
    #[instrument]
    fn can_round_trip_raw_json_bytes_without_reformatting() {
        let variable = Variable::new_for_test(DatatypeState::Creating);
        assert_eq!(&*variable.get_raw(), b"null");

        // Byte-for-byte preservation: key order and insignificant whitespace stay.
        let payload = br#"{ "b": 2, "a": 1 }"#;
        assert_eq!(&*variable.set_raw(payload).unwrap(), b"null");
        assert_eq!(&*variable.get_raw(), payload);

        // The previous value comes back as its stored bytes, not a reserialization.
        assert_eq!(&*variable.set_raw(b"[1,2,3]").unwrap(), payload);
    }

    #[test]
    #[instrument]
    fn can_reject_raw_json_that_is_not_exactly_one_value() {
        let variable = Variable::new_for_test(DatatypeState::Creating);
        for bad in [b"".as_slice(), b"{", b"1 2", b"null null", b"\xff"] {
            let error = variable.set_raw(bad).unwrap_err();
            assert_eq!(error, DatatypeError::ValueConversion(String::new()));
        }
        // Every rejected set is a no-op.
        assert_eq!(&*variable.get_raw(), b"null");
    }

    #[test]
    #[instrument]
    fn can_validate_raw_json_before_consulting_write_access() {
        // `set_raw` shares the write-access gate with `set`; the only ordering it
        // adds is that malformed JSON is rejected before that gate is reached.
        let variable = Variable::new_for_test(DatatypeState::Disabled);
        let error = variable.set_raw(b"{").unwrap_err();
        assert_eq!(error, DatatypeError::ValueConversion(String::new()));
    }

    #[test]
    #[instrument]
    fn can_use_transaction() {
        let variable = Variable::new_for_test(DatatypeState::Creating);
        variable.set("before").unwrap();

        let result = variable.transaction("success", |v| {
            v.set(&1_i64).unwrap();
            v.set(&2_i64).unwrap();
            Ok(())
        });
        assert!(result.is_ok());
        assert_eq!(variable.get::<i64>().unwrap(), 2);

        let result = variable.transaction("failure", |v| {
            v.set(&sample_profile()).unwrap();
            v.set(&Value::Null).unwrap();
            Err("failed".into())
        });
        assert!(result.is_err());
        assert_eq!(variable.get::<i64>().unwrap(), 2);
    }

    /// Builds two `Client`s sharing one non-realtime `LocalConnectivity`, so writes only
    /// propagate on an explicit `sync()`.
    fn two_clients_sharing() -> (Client, Client) {
        let connectivity = local_connectivity();
        connectivity.set_realtime(false);
        let (collection, _, _) = get_test_ids!();
        let client_a = Client::builder(collection.clone(), "client-a")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client_b = Client::builder(collection, "client-b")
            .with_connectivity(connectivity)
            .build()
            .unwrap();
        (client_a, client_b)
    }

    #[test]
    #[instrument]
    fn can_propagate_a_sequential_set_between_two_clients() {
        let (client1, client2) = two_clients_sharing();
        let (_, key, _) = get_test_ids!();

        let variable1 = client1
            .create_datatype(key.clone())
            .build_variable()
            .unwrap();
        variable1.sync().unwrap();

        let variable2 = client2.subscribe_datatype(key).build_variable().unwrap();
        variable2.sync().unwrap();
        assert_eq!(variable2.get::<Value>().unwrap(), Value::Null);

        variable1.set(&sample_profile()).unwrap();
        variable1.sync().unwrap();

        variable2.sync().unwrap();
        assert_eq!(variable2.get::<Profile>().unwrap(), sample_profile());
    }

    /// Has `client_a` and `client_b` each `Set` once from a shared baseline before either
    /// syncs, so both writes land on the same Lamport, then syncs them in the requested
    /// order. Returns the value each client observes afterward.
    fn run_concurrent_set(
        client_a: &Client,
        client_b: &Client,
        key: &str,
        a_syncs_first: bool,
    ) -> (String, String) {
        let variable_a = client_a.create_datatype(key).build_variable().unwrap();
        variable_a.sync().unwrap();
        let variable_b = client_b.subscribe_datatype(key).build_variable().unwrap();
        variable_b.sync().unwrap();

        variable_a.set("from-a").unwrap();
        variable_b.set("from-b").unwrap();

        if a_syncs_first {
            variable_a.sync().unwrap(); // pushes a's write
            variable_b.sync().unwrap(); // pushes b's write, pulls a's write
            variable_a.sync().unwrap(); // pulls b's write
        } else {
            variable_b.sync().unwrap();
            variable_a.sync().unwrap();
            variable_b.sync().unwrap();
        }

        (
            variable_a.get::<String>().unwrap(),
            variable_b.get::<String>().unwrap(),
        )
    }

    #[test]
    #[instrument]
    fn can_converge_via_cuid_tie_break_regardless_of_sync_order() {
        let (client_a, client_b) = two_clients_sharing();
        // Equal-Lamport ties resolve to the greater Cuid (Timestamp's derived Ord compares
        // lamport first, then cuid), independent of which client happens to sync first.
        let expected = if client_a.get_cuid() > client_b.get_cuid() {
            "from-a"
        } else {
            "from-b"
        };

        let (a_sees, b_sees) = run_concurrent_set(&client_a, &client_b, "key-a-first", true);
        assert_eq!(a_sees, expected);
        assert_eq!(b_sees, expected);

        let (a_sees, b_sees) = run_concurrent_set(&client_a, &client_b, "key-b-first", false);
        assert_eq!(a_sees, expected);
        assert_eq!(b_sees, expected);
    }

    #[test]
    #[instrument]
    fn can_ignore_a_stale_transaction_that_arrives_after_a_newer_value() {
        let (client_a, client_b) = two_clients_sharing();
        let (_, key, _) = get_test_ids!();

        let variable_a = client_a
            .create_datatype(key.clone())
            .build_variable()
            .unwrap();
        variable_a.sync().unwrap();
        let variable_b = client_b.subscribe_datatype(key).build_variable().unwrap();
        variable_b.sync().unwrap();

        // client_a's write stays pending (simulating a delayed push) while client_b races
        // ahead with two local writes of its own, ending at a strictly higher Lamport.
        variable_a.set("stale").unwrap();
        variable_b.set("intermediate").unwrap();
        variable_b.set("newer").unwrap();
        variable_b.sync().unwrap();

        // client_a pushes its stale write and, in the same round trip, pulls client_b's two
        // newer transactions; the higher Lamport wins at client_a too.
        variable_a.sync().unwrap();
        assert_eq!(variable_a.get::<String>().unwrap(), "newer");

        // client_b finally receives client_a's stale write; it must not overwrite "newer".
        variable_b.sync().unwrap();
        assert_eq!(variable_b.get::<String>().unwrap(), "newer");
    }

    #[test]
    #[instrument]
    fn can_catch_up_a_late_subscriber_to_the_current_value_and_timestamp() {
        let (client_a, client_b) = two_clients_sharing();
        let (_, key, _) = get_test_ids!();

        let variable_a = client_a
            .create_datatype(key.clone())
            .build_variable()
            .unwrap();
        variable_a.sync().unwrap();
        variable_a.set("first").unwrap();
        variable_a.sync().unwrap();
        variable_a.set("second").unwrap();
        variable_a.sync().unwrap();

        // client_b only subscribes now, after several Sets already happened, and still
        // catches up to the current value in a single sync — not just the first of the
        // accumulated writes.
        let variable_b = client_b.subscribe_datatype(key).build_variable().unwrap();
        variable_b.sync().unwrap();
        assert_eq!(variable_b.get::<String>().unwrap(), "second");

        // The absorbed timestamp is the real winning one, not the initial sentinel: a
        // further client_b Set is correctly ordered after client_a's history and
        // propagates back, proving the caught-up Lamport is usable, not stale.
        variable_b.set("third").unwrap();
        variable_b.sync().unwrap();
        variable_a.sync().unwrap();
        assert_eq!(variable_a.get::<String>().unwrap(), "third");
    }

    #[test]
    fn can_assert_send_and_sync_traits() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Variable>();
    }

    #[test]
    #[instrument]
    fn can_call_public_blanket_trait_methods() {
        let variable = Variable::new_for_test(DatatypeState::Subscribed);
        assert_eq!(variable.get_type(), DataType::Variable);
        assert_eq!(variable.get_key(), get_test_func_name!());
        assert_eq!(variable.get_state(), DatatypeState::Subscribed);
    }
}
