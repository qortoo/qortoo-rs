use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use crate::{
    ClientError, DataType, DatatypeState,
    clients::common::ClientCommon,
    datatypes::{datatype_set::DatatypeSet, option::DatatypeOption},
    errors::with_err_out,
    types::common::ArcStr,
};

pub struct DatatypeManager {
    common: Arc<ClientCommon>,
    datatypes: HashMap<ArcStr, DatatypeSet>,
}

impl DatatypeManager {
    pub fn new(common: Arc<ClientCommon>) -> Self {
        Self {
            datatypes: HashMap::new(),
            common,
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
        is_readonly: bool,
    ) -> Result<DatatypeSet, ClientError> {
        let arc_key: ArcStr = key.into();
        match self.datatypes.entry(arc_key.clone()) {
            Entry::Occupied(entry) => {
                let existing = entry.get();
                Err(with_err_out!(
                    ClientError::FailedToSubscribeOrCreateDatatype(format!(
                        "{type:?} '{key}' was demanded as {state:?}, but the client already has {:?} '{key}' as {:?}",
                        existing.get_type(),
                        existing.get_state()
                    ))
                ))
            }
            Entry::Vacant(entry) => {
                let dt = DatatypeSet::new(
                    r#type,
                    arc_key,
                    state,
                    self.common.clone(),
                    option,
                    is_readonly,
                );
                entry.insert(dt.clone());
                Ok(dt)
            }
        }
    }
}

#[cfg(test)]
mod tests_datatype_manager {
    use tracing::instrument;

    use crate::{
        ClientError, DataType, DatatypeState,
        clients::{common::new_client_common, datatype_manager::DatatypeManager},
    };

    #[test]
    #[instrument]
    fn can_use_subscribe_or_create_datatype() {
        let mut dm = DatatypeManager::new(new_client_common!());
        let res1 = dm.subscribe_or_create_datatype(
            "k1",
            DataType::Counter,
            DatatypeState::DueToCreate,
            Default::default(),
            false,
        );
        assert!(res1.is_ok());
        let dt1 = res1.unwrap();
        assert_eq!(dt1.get_state(), DatatypeState::DueToCreate);
        assert_eq!(dt1.get_type(), DataType::Counter);

        let res2 = dm.subscribe_or_create_datatype(
            "k1",
            DataType::Map,
            DatatypeState::DueToCreate,
            Default::default(),
            false,
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
            false,
        );
        assert_eq!(
            res3.err().unwrap(),
            ClientError::FailedToSubscribeOrCreateDatatype("".into())
        );
    }
}
