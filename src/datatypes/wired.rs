use std::sync::Arc;

use parking_lot::RwLock;

use crate::datatypes::{common::Attribute, mutable::MutableDatatype};

pub struct WiredDatatype {
    pub mutable: Arc<RwLock<MutableDatatype>>,
    pub attr: Arc<Attribute>,
}

impl WiredDatatype {
    pub fn push_transaction(&self) {
        let _mutable = self.mutable.write();
        // do something with mutable
    }
}
