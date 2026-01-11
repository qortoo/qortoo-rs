use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

use crate::{
    DataType,
    datatypes::option::DatatypeOption,
    types::{
        common::{ArcStr, ResourceID},
        uid::{Cuid, Duid},
    },
};

macro_rules! datatype_instrument {
    ($(#[$attr:meta])* $vis:vis fn $name:ident $($rest:tt)*) => {
        $(#[$attr])*
        #[tracing::instrument(skip_all,
            fields(
                qortoo.col=%self.datatype.attr.client_common.collection,
                qortoo.cl=%self.datatype.attr.client_common.alias,
                qortoo.cuid=%self.datatype.attr.client_common.cuid,
                qortoo.dt=%self.datatype.attr.key,
                qortoo.duid=%self.datatype.attr.get_duid(),
            )
        )]
        $vis fn $name $($rest)*
    };
}

macro_rules! internal_datatype_instrument {
    ($span_name:expr, $(#[$attr:meta])* $vis:vis fn $name:ident $($rest:tt)*) => {
        $(#[$attr])*
        #[tracing::instrument(skip_all,
            name = $span_name,
            fields(
                qortoo.col=%self.attr.client_common.collection,
                qortoo.cl=%self.attr.client_common.alias,
                qortoo.cuid=%self.attr.client_common.cuid,
                qortoo.dt=%self.attr.key,
                qortoo.duid=%self.attr.get_duid(),
            )
        )]
        $vis fn $name $($rest)*
    };
}

pub(crate) use datatype_instrument;
pub(crate) use internal_datatype_instrument;

pub struct Attribute {
    pub key: ArcStr,
    pub r#type: DataType,
    pub duid: RwLock<Duid>,
    pub client_common: Arc<ClientCommon>,
    pub option: Arc<DatatypeOption>,
    pub is_readonly: bool,
}

impl Debug for Attribute {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Datatype")
            .field("client", &self.client_common)
            .field("key", &self.key)
            .field("type", &self.r#type)
            .field("duid", &self.duid.read().to_string())
            .field("option", &self.option)
            .finish()
    }
}

impl Attribute {
    pub fn new(
        key: ArcStr,
        r#type: DataType,
        client_common: Arc<ClientCommon>,
        option: DatatypeOption,
        is_readonly: bool,
    ) -> Self {
        Self {
            key,
            r#type,
            duid: RwLock::new(Duid::new()),
            client_common,
            option: Arc::new(option),
            is_readonly,
        }
    }

    pub fn resource_id(&self) -> ResourceID {
        format!("{}/{}", self.client_common.collection, self.key)
    }

    #[cfg(test)]
    pub fn new_for_test_with_connectivity(
        mut paths: std::collections::VecDeque<String>,
        r#type: DataType,
        connectivity: Arc<dyn crate::connectivity::Connectivity>,
    ) -> Arc<Self> {
        let key = paths.pop_back().unwrap_or(format!("{type}")).into();
        let client_alias = paths.pop_back().unwrap_or("client".into()).into();
        let collection = paths.pop_back().unwrap_or("collection".to_owned()).into();
        let client_common = ClientCommon::new_arc(collection, client_alias, connectivity);
        Arc::new(Self {
            key,
            r#type,
            duid: RwLock::new(Duid::new()),
            client_common,
            option: Default::default(),
            is_readonly: false,
        })
    }

    #[cfg(test)]
    pub fn new_for_test(paths: std::collections::VecDeque<String>, r#type: DataType) -> Arc<Self> {
        Self::new_for_test_with_connectivity(
            paths,
            r#type,
            Arc::new(crate::connectivity::null_connectivity::NullConnectivity::new()),
        )
    }

    pub fn cuid(&self) -> Cuid {
        self.client_common.cuid.clone()
    }

    pub fn get_duid(&self) -> Duid {
        self.duid.read().clone()
    }

    pub fn set_duid(&self, new_duid: Duid) {
        *self.duid.write() = new_duid;
    }
}

#[cfg(test)]
macro_rules! new_attribute {
    ($enum_variant:path) => {{
        let paths = crate::utils::path::caller_path!();
        crate::datatypes::common::Attribute::new_for_test(paths, $enum_variant)
    }};
}

#[cfg(test)]
macro_rules! new_attribute_with_connectivity {
    ($enum_variant:path, $connectivity:expr) => {{
        let paths = crate::utils::path::caller_path!();
        crate::datatypes::common::Attribute::new_for_test_with_connectivity(
            paths,
            $enum_variant,
            $connectivity,
        )
    }};
}

#[cfg(test)]
pub(crate) use new_attribute;
#[cfg(test)]
pub(crate) use new_attribute_with_connectivity;
use parking_lot::RwLock;

use crate::clients::common::ClientCommon;

pub enum ReturnType {
    None,
    Counter(i64),
}

#[cfg(test)]
mod tests_attribute {

    use tracing::info;

    use crate::{DataType, types::uid::Duid, utils::path::caller_path};

    #[test]
    fn can_new_attribute_for_test() {
        let attr = new_attribute!(DataType::Counter);
        info!("{:?}", attr);
        let mut caller_path = caller_path!();
        assert_eq!(attr.r#type, DataType::Counter);
        assert_eq!(attr.key.as_ref(), caller_path.pop_back().unwrap());
        assert_ne!(
            attr.client_common.alias.to_string(),
            caller_path.pop_back().unwrap()
        );
        assert_eq!(
            attr.client_common.collection.to_string(),
            caller_path.pop_back().unwrap()
        );
        assert_eq!(
            format!("{}/{}", attr.client_common.collection, attr.key),
            attr.resource_id()
        );
    }

    #[test]
    fn can_use_get_and_set_duid() {
        let attr = new_attribute!(DataType::Counter);
        let new_duid = Duid::new();
        attr.set_duid(new_duid.clone());
        let got_duid = attr.get_duid();
        assert_eq!(new_duid, got_duid);
        assert_eq!(new_duid.as_ptr(), got_duid.as_ptr());
        info!("{}: {}", new_duid, new_duid.as_ptr() as u64);
        info!("{}: {}", got_duid, got_duid.as_ptr() as u64);
    }
}
