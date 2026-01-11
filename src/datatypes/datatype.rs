use crate::{
    DataType, DatatypeError, DatatypeState, datatypes::transactional::TransactionalDatatype,
};

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
/// use qortoo::Client;
/// use qortoo::{Counter, Datatype};
/// use qortoo::{DatatypeState, DataType};
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
    fn sync(&self) -> Result<(), DatatypeError>;
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

    fn sync(&self) -> Result<(), DatatypeError> {
        self.get_core().sync()
    }
}

#[cfg(test)]
mod tests_datatype_trait {
    use tracing::instrument;

    use crate::{
        Client, DataType, DatatypeError, DatatypeState,
        connectivity::local_connectivity::LocalConnectivity,
        datatypes::{
            common::new_attribute, datatype::Datatype, transactional::TransactionalDatatype,
        },
        errors::push_pull::ClientPushPullError,
        utils::path::get_test_func_name,
    };

    #[test]
    #[instrument]
    fn can_call_datatype_trait_methods() {
        let attr = new_attribute!(DataType::Counter);
        let key = attr.key.as_ref();
        let data = TransactionalDatatype::new_arc(attr.clone(), DatatypeState::DueToCreate);
        assert_eq!(data.get_key(), key);
        assert_eq!(data.get_type(), DataType::Counter);
        assert_eq!(data.get_state(), DatatypeState::DueToCreate);
        assert_eq!(data.get_server_version(), 0);
        assert_eq!(data.get_client_version(), 0);
        assert_eq!(data.get_synced_client_version(), 0);
    }

    #[test]
    #[instrument]
    fn can_use_sync_method() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let resource_id = format!("{}/{}", module_path!(), get_test_func_name!());
        let client1 = Client::builder(module_path!(), module_path!())
            .with_connectivity(connectivity.clone())
            .build();
        let counter1 = client1
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();

        let interceptor1 = connectivity
            .get_wired_interceptor(&resource_id, &client1.get_cuid())
            .unwrap();

        // produce push_pull error
        interceptor1.set_after_pull(|_pull| Err(ClientPushPullError::ExceedMaxMemSize));

        assert_eq!(
            counter1.sync().unwrap_err(),
            DatatypeError::FailedToSync(ClientPushPullError::ExceedMaxMemSize)
        );
        assert_eq!(counter1.get_state(), DatatypeState::DueToCreate);

        // make a success case
        interceptor1.set_after_pull(|_pull| Ok(()));
        assert!(counter1.sync().is_ok());
        assert_eq!(counter1.get_state(), DatatypeState::Subscribed);
    }
}
