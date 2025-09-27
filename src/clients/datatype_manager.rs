use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use crate::{
    ClientError, DataType, DatatypeState,
    clients::client::ClientInfo,
    datatypes::{datatype_set::DatatypeSet, option::DatatypeOption},
    errors::err,
};

pub struct DatatypeManager {
    info: Arc<ClientInfo>,
    datatypes: HashMap<String, DatatypeSet>,
}

impl DatatypeManager {
    pub fn new(client_info: Arc<ClientInfo>) -> Self {
        Self {
            datatypes: HashMap::new(),
            info: client_info,
        }
    }

    pub fn get_datatype(&self, key: &str) -> Option<DatatypeSet> {
        self.datatypes.get(key).cloned()
    }

    pub fn subscribe_or_create_datatype(
        &mut self,
        key: &str,
        r#type: DataType,
        state: DatatypeState,
        option: DatatypeOption,
    ) -> Result<DatatypeSet, ClientError> {
        match self.datatypes.entry(key.to_owned()) {
            Entry::Occupied(entry) => {
                let existing = entry.get();
                if existing.get_type() != r#type || existing.get_state() != state {
                    return Err(err!(
                        ClientError::FailedToSubscribeOrCreateDatatype,
                        format!(
                            "{type:?} is demanded as {state:?}, but the clients has {:?} for '{key}' as {:?}",
                            existing.get_type(),
                            existing.get_state()
                        )
                    ));
                }
                Ok(existing.clone())
            }
            Entry::Vacant(entry) => {
                let dt = DatatypeSet::new(r#type, key, state, self.info.clone(), option);
                entry.insert(dt.clone());
                Ok(dt)
            }
        }
    }
}

#[cfg(test)]
mod tests_datatype_manager {
    use crate::{ClientError, DataType, DatatypeState, clients::datatype_manager::DatatypeManager};

    #[test]
    fn can_use_subscribe_or_create_datatype() {
        let mut dm = DatatypeManager::new(Default::default());
        let res1 = dm.subscribe_or_create_datatype(
            "k1",
            DataType::Counter,
            DatatypeState::DueToCreate,
            Default::default(),
        );
        assert!(res1.is_ok());
        let dt1 = res1.unwrap();
        assert_eq!(dt1.get_type(), DataType::Counter);
        assert_eq!(dt1.get_state(), DatatypeState::DueToCreate);

        let res2 = dm.subscribe_or_create_datatype(
            "k1",
            DataType::List,
            DatatypeState::DueToCreate,
            Default::default(),
        );
        assert_eq!(
            res2.err().unwrap(),
            ClientError::FailedToSubscribeOrCreateDatatype("".into())
        );

        let res3 = dm.subscribe_or_create_datatype(
            "k1",
            DataType::Counter,
            DatatypeState::DueToSubscribeOrCreate,
            Default::default(),
        );
        assert_eq!(
            res3.err().unwrap(),
            ClientError::FailedToSubscribeOrCreateDatatype("".into())
        );

        let res4 = dm.subscribe_or_create_datatype(
            "k1",
            DataType::Counter,
            DatatypeState::DueToCreate,
            Default::default(),
        );
        assert!(res4.is_ok());
        let dt4 = res4.unwrap();
        assert_eq!(dt4.get_state(), DatatypeState::DueToCreate);
    }
}
