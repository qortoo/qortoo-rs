use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

use crate::{DataType, datatypes::option::DatatypeOption, types::uid::Duid};

macro_rules! datatype_instrument {
    ($(#[$attr:meta])* $vis:vis fn $name:ident $($rest:tt)*) => {
        $(#[$attr])*
        #[tracing::instrument(skip_all,
            fields(
                syncyam.col=%self.datatype.attr.client_common.collection,
                syncyam.cl=%self.datatype.attr.client_common.alias,
                syncyam.cuid=%self.datatype.attr.client_common.cuid,
                syncyam.dt=%self.datatype.attr.key,
                syncyam.duid=%self.datatype.attr.duid,
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
                syncyam.col=%self.attr.client_common.collection,
                syncyam.cl=%self.attr.client_common.alias,
                syncyam.cuid=%self.attr.client_common.cuid,
                syncyam.dt=%self.attr.key,
                syncyam.duid=%self.attr.duid,
            )
        )]
        $vis fn $name $($rest)*
    };
}

pub(crate) use datatype_instrument;
pub(crate) use internal_datatype_instrument;

pub struct Attribute {
    pub key: String,
    pub r#type: DataType,
    pub duid: Duid,
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
            .field("duid", &self.duid.to_string())
            .field("option", &self.option)
            .finish()
    }
}

impl Attribute {
    pub fn new(
        key: String,
        r#type: DataType,
        client_common: Arc<ClientCommon>,
        option: DatatypeOption,
        is_readonly: bool,
    ) -> Self {
        Self {
            key,
            r#type,
            duid: Duid::new(),
            client_common,
            option: Arc::new(option),
            is_readonly,
        }
    }

    pub fn resource_id(&self) -> String {
        format!("{}/{}", self.client_common.collection, self.key)
    }

    #[cfg(test)]
    pub fn new_for_test(
        mut paths: std::collections::VecDeque<String>,
        r#type: DataType,
    ) -> Arc<Self> {
        use crate::connectivity::null_connectivity::NullConnectivity;

        let key = paths.pop_back().unwrap_or(format!("{type}"));
        let client_alias = paths
            .pop_back()
            .unwrap_or("client".to_owned())
            .into_boxed_str();
        let collection = paths
            .pop_back()
            .unwrap_or("collection".to_owned())
            .into_boxed_str();
        let client_common =
            ClientCommon::new_arc(collection, client_alias, Arc::new(NullConnectivity::new()));
        Arc::new(Self {
            key,
            r#type,
            duid: Duid::new(),
            client_common,
            option: Default::default(),
            is_readonly: false,
        })
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
pub(crate) use new_attribute;

use crate::clients::common::ClientCommon;

pub enum ReturnType {
    None,
    Counter(i64),
}

#[cfg(test)]
mod tests_attribute {
    use tracing::info;

    use crate::{DataType, utils::path::caller_path};

    #[test]
    fn can_new_attribute_for_test() {
        let attr = new_attribute!(DataType::Counter);
        info!("{:?}", attr);
        let mut caller_path = caller_path!();
        assert_eq!(attr.r#type, DataType::Counter);
        assert_eq!(attr.key, caller_path.pop_back().unwrap());
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
}
