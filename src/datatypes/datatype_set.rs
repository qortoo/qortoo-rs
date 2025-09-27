use std::sync::Arc;

use crate::{
    Counter, DataType, Datatype, DatatypeState,
    clients::client::ClientInfo,
    datatypes::{common::Attribute, option::DatatypeOption, transactional::TransactionalDatatype},
};

/// A typed wrapper for concrete datatypes managed by the client.
///
/// `DatatypeSet` allows returning a single enum while preserving
/// type information and shared behavior across datatypes.
#[derive(Clone)]
pub enum DatatypeSet {
    Counter(Counter),
}

impl DatatypeSet {
    /// Returns the internal datatype in this wrapper, e.g. `DataType::Counter`
    pub fn get_type(&self) -> DataType {
        match self {
            DatatypeSet::Counter(_) => DataType::Counter,
        }
    }

    /// Returns [`DatatypeState`] of the internal datatype in this wrapper,
    /// e.g., `DatatypeState::DueToCreate`
    pub fn get_state(&self) -> DatatypeState {
        match self {
            DatatypeSet::Counter(cnt) => cnt.get_state(),
        }
    }

    /// Creates a new [`DatatypeSet`] instance for the given `type` and `key`.
    ///
    /// This is primarily used by the client internals to construct
    /// a concrete datatype variant tied to a specific client context.
    pub(crate) fn new(
        r#type: DataType,
        key: &str,
        state: DatatypeState,
        client_info: Arc<ClientInfo>,
        option: DatatypeOption,
    ) -> Self {
        let attr = Arc::new(Attribute::new(key.to_owned(), r#type, client_info, option));
        let datatype = TransactionalDatatype::new_arc(attr, state);
        match r#type {
            DataType::Counter => DatatypeSet::Counter(Counter::new(datatype)),
            _ => {
                todo!()
            }
        }
    }
}

#[cfg(test)]
mod tests_datatype_set {
    use crate::{
        Counter, DataType, Datatype, DatatypeState,
        datatypes::{
            datatype::DatatypeBlanket, datatype_set::DatatypeSet,
            transactional::TransactionalDatatype,
        },
    };

    #[test]
    fn can_clone_datatype_set() {
        let ds1 = DatatypeSet::new(
            DataType::Counter,
            "k1",
            DatatypeState::DueToCreate,
            Default::default(),
            Default::default(),
        );
        let ds2 = ds1.clone();
        let DatatypeSet::Counter(cnt1) = ds1;
        let DatatypeSet::Counter(cnt2) = ds2;

        // Cloned DatatypeSet contains a cloned Counter (same variant, same key)
        assert_eq!(cnt1.get_key(), cnt2.get_key());
        assert_eq!(cnt1.get_type(), cnt2.get_type());

        // Verify the cloned Counter operates correctly and shares state
        assert_eq!(0, cnt1.get_value());
        assert_eq!(2, cnt2.increase_by(2));
        assert_eq!(2, cnt1.get_value());

        // Verify the cloned Counter is different from the original
        let ptr1: *const Counter = &cnt1;
        let ptr2: *const Counter = &cnt2;
        assert_ne!(ptr1, ptr2);

        // Verify the cloned Counter has the same TransactionalDatatype as the original
        let ptr1: *const TransactionalDatatype = cnt1.get_core();
        let ptr2: *const TransactionalDatatype = cnt2.get_core();
        assert_eq!(ptr1, ptr2);
    }
}
