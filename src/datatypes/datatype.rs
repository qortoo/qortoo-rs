use crate::{DataType, DatatypeState, datatypes::transactional::TransactionalDatatype};

/// The `Datatype` trait defines the common interface for all
/// conflict-free datatypes (e.g., Counter, Register, Document).
///
/// Each datatype exposes:
/// - a **key**: a unique identifier used to distinguish instances in a collection,
/// - a **type**: an enum variant of [`DataType`] describing the kind of datatype,
/// - a **state**: a [`DatatypeState`] indicating the current lifecycle/state of this datatype.
///
///
/// # Example
/// ```
/// use syncyam::Client;
/// use syncyam::{Counter, Datatype};
/// use syncyam::{DatatypeState, DataType};
/// let client = Client::builder("doc-example", "Datatype-trait").build();
/// let counter = client.create_datatype("test-counter".to_string()).build_counter().unwrap();
/// assert_eq!(counter.get_key(), "test-counter");
/// assert_eq!(counter.get_type(), DataType::Counter);
/// assert_eq!(counter.get_state(), DatatypeState::DueToCreate);
/// ```
pub trait Datatype {
    /// returns a unique identifier used to distinguish instances in a collection.
    fn get_key(&self) -> &str;
    /// returns an enum variant of [`DataType`] describing the kind of this datatype.
    fn get_type(&self) -> DataType;
    /// returns a [`DatatypeState`] indicating the current lifecycle/status of this datatype.
    fn get_state(&self) -> DatatypeState;
    fn get_server_version(&self) -> u64;
    fn get_client_version(&self) -> u64;
    fn get_synced_client_version(&self) -> u64;
}

pub trait DatatypeBlanket {
    fn get_core(&self) -> &TransactionalDatatype;
}

impl<T> Datatype for T
where
    T: DatatypeBlanket,
{
    fn get_key(&self) -> &str {
        self.get_core().get_key()
    }

    fn get_type(&self) -> DataType {
        self.get_core().get_type()
    }

    fn get_state(&self) -> DatatypeState {
        self.get_core().get_state()
    }

    fn get_server_version(&self) -> u64 {
        self.get_core().get_server_version()
    }

    fn get_client_version(&self) -> u64 {
        self.get_core().get_client_version()
    }

    fn get_synced_client_version(&self) -> u64 {
        self.get_core().get_synced_client_version()
    }
}

#[cfg(test)]
mod tests_datatype_trait {
    use tracing::instrument;

    use crate::{
        DataType, DatatypeState,
        datatypes::{
            common::new_attribute, datatype::Datatype, transactional::TransactionalDatatype,
        },
    };

    #[test]
    #[instrument]
    fn can_call_datatype_trait_functions() {
        let attr = new_attribute!(DataType::Counter);
        let key = attr.key.clone();
        let data = TransactionalDatatype::new_arc(attr, DatatypeState::DueToCreate);
        assert_eq!(data.get_key(), key);
        assert_eq!(data.get_type(), DataType::Counter);
        assert_eq!(data.get_state(), DatatypeState::DueToCreate);
        assert_eq!(data.get_server_version(), 0);
        assert_eq!(data.get_client_version(), 0);
        assert_eq!(data.get_synced_client_version(), 0);
    }
}
