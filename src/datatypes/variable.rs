use std::sync::Arc;

use serde::{Serialize, de::DeserializeOwned};
use tracing::trace;

use crate::{
    DatatypeError, IntoString,
    datatypes::{
        common::{ReturnType, datatype_instrument},
        datatype::DatatypeBlanket,
        transactional::{TransactionContext, TransactionalDatatype},
        value::{decode_json_value, encode_json_value},
    },
    errors::{BoxedError, datatypes::InternalReason},
    operations::Operation,
};

/// A variable is a conflict-free datatype that stores a single JSON value with
/// last-writer-wins semantics.
///
/// The stored value is always exactly one JSON value and starts as JSON `null`.
/// Every write carries a logical timestamp, and writes made concurrently on
/// different replicas converge to the one with the greatest timestamp, so every
/// replica ends up holding the same value.
///
/// Values are exchanged with user code through `serde`: [`set`](Self::set) stores
/// any serializable value and [`get`](Self::get) decodes the current value into
/// the requested type. A value that cannot be converted surfaces as
/// [`DatatypeError::ValueConversion`], leaving the variable unchanged.
///
/// Like every datatype, a `Variable` shares the lifecycle, synchronization, and
/// handler API of [`Datatype`](crate::Datatype).
#[derive(Clone)]
pub struct Variable {
    datatype: Arc<TransactionalDatatype>,
    tx_ctx: Arc<TransactionContext>,
}

impl Variable {
    // dead_code is allowed until datatype registration constructs Variable outside tests.
    #[cfg_attr(not(test), allow(dead_code))]
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
    /// Sets the variable to a new value.
    ///
    /// The value is encoded to JSON and recorded as a last-writer-wins write
    /// stamped with the next logical timestamp.
    ///
    /// Returns the value the variable held just before this set; the first set
    /// returns the initial JSON `null`. The previous value is returned as
    /// dynamic JSON because each set may store a different JSON type.
    ///
    /// # Arguments
    ///
    /// * `value` - Any serializable value; it is stored as one JSON value
    ///
    /// # Returns
    ///
    /// The previous JSON value, [`DatatypeError::ValueConversion`] if the value
    /// cannot be encoded to JSON, or [`DatatypeError::NotWritable`] /
    /// [`DatatypeError::ReadonlyViolation`] if writes are not allowed. The
    /// variable is left unchanged on every error.
    pub fn set<T: Serialize + ?Sized>(
        &self,
        value: &T,
    ) -> Result<serde_json::Value, DatatypeError> {
        let encoded = encode_json_value(value)?;
        let op = Operation::new_variable_set(encoded);

        let ret = self
            .datatype
            .execute_local_operation_as_tx(self.tx_ctx.clone(), op)?;
        trace!("set -> {ret:?}");
        match ret {
            ReturnType::Variable(previous) => decode_json_value(&previous),
            _ => Err(InternalReason::ExecuteOperation("unexpected return type".into()).into_error())
        }
    }}

    /// Gets the current value decoded into the requested type.
    ///
    /// This is a local read: it creates no operation and does not change the
    /// datatype version, so it is allowed in every state, readonly datatypes
    /// included. The initial value and an explicit [`set`](Self::set) of
    /// JSON `null` both read as JSON `null`; use a nullable destination such as
    /// `Option<T>` or `serde_json::Value` to accept it. Decoding JSON `null`
    /// into a non-nullable type returns [`DatatypeError::ValueConversion`].
    ///
    /// # Returns
    ///
    /// The current value decoded as `T`, or
    /// [`DatatypeError::ValueConversion`] if the stored JSON does not fit `T`
    pub fn get<T: DeserializeOwned>(&self) -> Result<T, DatatypeError> {
        let value = {
            let mutable = self.datatype.mutable.read();
            mutable
                .crdt
                .as_variable()
                .expect("variable datatype must contain a variable crdt")
                .shared_value()
        };
        decode_json_value(&value)
    }

    datatype_instrument! {
    /// Executes multiple operations atomically within a transaction.
    ///
    /// If the transaction function returns an error, every [`set`](Self::set)
    /// within the transaction is rolled back, restoring the value and its
    /// timestamp from before the transaction.
    ///
    /// # Arguments
    ///
    /// * `tag` - A descriptive label for the transaction
    /// * `tx_func` - Function containing the operations to execute atomically
    ///
    /// # Returns
    ///
    /// `Ok(())` if the transaction committed,
    /// [`DatatypeError::TransactionFailed`] if `tx_func` returned an error, or
    /// [`DatatypeError::NotWritable`] / [`DatatypeError::ReadonlyViolation`] if
    /// writes are not allowed
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
        Client, DataType, DatatypeError, DatatypeState, LocalConnectivity,
        datatypes::{datatype::Datatype, variable::Variable},
        utils::test_utils::{get_test_collection_name, get_test_func_name, get_test_ids},
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

        let error = variable.set(&map).unwrap_err();
        assert_eq!(error, DatatypeError::ValueConversion(String::new()));
        assert_eq!(variable.get::<Value>().unwrap(), Value::Null);
        // The failed set produced no operation: the next set still sees the initial null.
        assert_eq!(variable.set(&1_i64).unwrap(), Value::Null);
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

    #[test]
    #[instrument]
    fn can_propagate_a_sequential_set_between_two_clients() {
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
