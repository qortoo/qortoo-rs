use std::sync::Arc;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    BoxedError, DatatypeError, IntoString,
    datatypes::{
        common::{ReturnType, datatype_instrument},
        crdts::MapCrdt,
        datatype::DatatypeBlanket,
        transactional::{TransactionContext, TransactionalDatatype},
    },
    errors::{datatypes::InternalReason, messages::datatype as msg},
    operations::Operation,
    utils::json::{decode_json_value, encode_json_value, validate_json_str},
};

/// A conflict-free datatype that stores JSON values by key.
///
/// A new map is empty. Writes to the same key converge to the change with the
/// greatest logical timestamp. Keys accept any UTF-8 string, including the empty
/// string. A stored JSON `null` counts as a present entry.
///
/// Use [`DatatypeBuilder::build_map`](crate::DatatypeBuilder::build_map) to obtain
/// a handle and [`Datatype`](crate::Datatype) for lifecycle, synchronization, and
/// handlers. Reads are local and available in every lifecycle state, including
/// on read-only handles. See the [Map guide](https://github.com/qortoo/qortoo-rs/blob/main/docs/map.md).
///
/// # Examples
///
/// ```
/// use qortoo::Client;
/// use serde_json::Value;
///
/// let client = Client::builder("doc-example", "map")
///     .build_and_open()
///     .unwrap();
/// let map = client.create_datatype("settings").build_map().unwrap();
///
/// assert_eq!(map.get::<Value>("theme").unwrap(), None);
/// assert_eq!(map.put("theme", &Value::Null).unwrap(), None);
/// assert_eq!(map.get::<Value>("theme").unwrap(), Some(Value::Null));
///
/// let json = br#" "dark" "#;
/// let previous = map.put_raw("theme", json).unwrap();
/// assert_eq!(previous.as_deref(), Some(b"null".as_slice()));
/// assert_eq!(map.remove_raw("theme").unwrap().as_deref(), Some(json.as_slice()));
/// assert!(map.is_empty());
/// ```
#[derive(Clone)]
pub struct Map {
    datatype: Arc<TransactionalDatatype>,
    tx_ctx: Arc<TransactionContext>,
}

impl Map {
    pub(crate) fn new(datatype: Arc<TransactionalDatatype>) -> Self {
        Self {
            datatype,
            tx_ctx: Default::default(),
        }
    }

    datatype_instrument! {
    /// Stores a serializable value as JSON.
    ///
    /// Returns the previous value as a [`Value`], or `None` if the key was absent.
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError::ValueConversion`] if the new value cannot be
    /// encoded as JSON or the previous value cannot be decoded as a [`Value`].
    /// Both conversions happen before the entry is changed. Use
    /// [`put_raw`](Self::put_raw) to replace a value while returning its original bytes.
    /// Also returns an error if the write is rejected or cannot commit locally.
    pub fn put<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
    ) -> Result<Option<Value>, DatatypeError> {
        let encoded = encode_json_value(value)?;
        validate_json_str(&encoded)?;
        self.write_with_previous_value(key, Operation::new_map_put(key.into(), encoded))
    }}

    datatype_instrument! {
    /// Stores exactly one UTF-8 JSON value, preserving the supplied bytes verbatim.
    ///
    /// Returns the previous value as its original bytes, or `None` if the key was
    /// absent. Whitespace, property order, and numeric formatting are preserved.
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError::ValueConversion`] for invalid input before
    /// changing the entry. Also returns an error if the write is rejected or
    /// cannot commit locally.
    pub fn put_raw(&self, key: &str, json: &[u8]) -> Result<Option<Box<[u8]>>, DatatypeError> {
        let encoded = validate_json_str(json)?.as_bytes().into();
        self.write_raw(Operation::new_map_put(key.into(), encoded))
    }}

    /// Returns the value at `key` decoded as `T`, or `None` if the key is absent.
    ///
    /// A present JSON `null` decodes to `Some(None)` when `T` is `Option<U>`.
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError::ValueConversion`] if a present value cannot be decoded as `T`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, DatatypeError> {
        let value = self.read(|crdt| crdt.shared_value(key));
        value.as_deref().map(decode_json_value).transpose()
    }

    /// Returns a copy of the stored JSON bytes, or `None` if the key is absent.
    pub fn get_raw(&self, key: &str) -> Option<Box<[u8]>> {
        self.read(|crdt| crdt.shared_value(key))
            .as_deref()
            .map(Box::from)
    }

    datatype_instrument! {
    /// Removes a key and returns its previous value as a [`Value`].
    ///
    /// Returns `None` for an absent key. Once write access is checked, removing
    /// an absent key records no operation.
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError::ValueConversion`] if the previous value cannot be
    /// decoded as a [`Value`], leaving the entry unchanged. Use
    /// [`remove_raw`](Self::remove_raw) to remove a value while returning its
    /// original bytes. Also returns an error if the write is rejected or cannot commit locally.
    pub fn remove(&self, key: &str) -> Result<Option<Value>, DatatypeError> {
        self.write_with_previous_value(key, Operation::new_map_remove(key.into()))
    }}

    datatype_instrument! {
    /// Removes a key and returns its previous value as its original JSON bytes.
    ///
    /// Returns `None` for an absent key. Once write access is checked, removing
    /// an absent key records no operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the write is rejected or cannot commit locally.
    pub fn remove_raw(&self, key: &str) -> Result<Option<Box<[u8]>>, DatatypeError> {
        self.write_raw(Operation::new_map_remove(key.into()))
    }}

    fn write_with_previous_value(
        &self,
        key: &str,
        operation: Operation,
    ) -> Result<Option<Value>, DatatypeError> {
        self.datatype
            .with_local_operation_as_tx(self.tx_ctx.clone(), |mutable| {
                // Convert the previous value under the same write lock, before mutating it.
                let previous = mutable
                    .crdt
                    .as_map()
                    .expect("map datatype must contain a map crdt")
                    .shared_value(key);
                let previous = previous.as_deref().map(decode_json_value).transpose()?;
                mutable.execute_local_operation(operation)?;
                Ok(previous)
            })
    }

    fn write_raw(&self, operation: Operation) -> Result<Option<Box<[u8]>>, DatatypeError> {
        match self
            .datatype
            .execute_local_operation_as_tx(self.tx_ctx.clone(), operation)?
        {
            ReturnType::Map(previous) => Ok(previous.as_deref().map(Box::from)),
            _ => Err(
                InternalReason::ExecuteOperation(msg::UNEXPECTED_RETURN_TYPE.into()).into_error(),
            ),
        }
    }

    /// Returns the number of keys with a value, including JSON `null` values.
    pub fn len(&self) -> usize {
        self.read(MapCrdt::len)
    }

    /// Returns `true` when no keys have a value.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns whether the key is present, including when its value is JSON `null`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.read(|crdt| crdt.contains_key(key))
    }

    /// Returns a JSON object with the entries present at the time of the read.
    ///
    /// An empty map returns an empty object. The object contains the decoded
    /// values. To obtain the original JSON bytes for an entry, use [`get_raw`](Self::get_raw).
    ///
    /// # Errors
    ///
    /// Returns [`DatatypeError::ValueConversion`] if a stored raw value cannot
    /// be decoded as a [`Value`].
    pub fn to_json(&self) -> Result<Value, DatatypeError> {
        let snapshot = self.read(MapCrdt::fork_for_batch);
        snapshot.to_json()
    }

    fn read<R>(&self, read: impl FnOnce(&MapCrdt) -> R) -> R {
        let mutable = self.datatype.core.mutable.read();
        read(
            mutable
                .crdt
                .as_map()
                .expect("map datatype must contain a map crdt"),
        )
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
    /// ```
    /// use qortoo::{Client, DatatypeError};
    ///
    /// let client = Client::builder("doc-example", "map-transaction")
    ///     .build_and_open()
    ///     .unwrap();
    /// let map = client.create_datatype("settings").build_map().unwrap();
    ///
    /// let result = map.transaction("update settings", |settings| {
    ///     settings.put("theme", "dark")?;
    ///     settings.put_raw("alerts", b"true false")?;
    ///     Ok(())
    /// });
    /// assert!(matches!(result, Err(DatatypeError::TransactionFailed(_))));
    /// assert!(map.is_empty());
    /// ```
    pub fn transaction<T>(&self, tag: impl IntoString, tx_func: T) -> Result<(), DatatypeError>
    where
        T: FnOnce(Self) -> Result<(), BoxedError> + Send + Sync + 'static,
    {
        self.datatype.check_writable()?;
        let tx_ctx = Arc::new(TransactionContext::new(tag));
        let map = Self {
            datatype: self.datatype.clone(),
            tx_ctx: tx_ctx.clone(),
        };
        self.datatype.do_transaction(tx_ctx, move || {
            tx_func(map).map_err(|error| DatatypeError::TransactionFailed(error.to_string()))
        })
    }}
}

impl DatatypeBlanket for Map {
    fn get_core(&self) -> &TransactionalDatatype {
        self.datatype.as_ref()
    }
}
