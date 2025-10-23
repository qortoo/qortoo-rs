use std::sync::Arc;

use parking_lot::RwLock;

use crate::datatypes::{common::Attribute, mutable::MutableDatatype};

pub struct WiredDatatype {
    pub mutable: Arc<RwLock<MutableDatatype>>,
    pub attr: Arc<Attribute>,
}

impl WiredDatatype {
    pub fn push_pull(&self) {
        let mut mutable = self.mutable.write();
        mutable.push_pull();
    }
}

impl MutableDatatype {
    fn push_pull(&mut self) {
        // todo: implement push_pull logic
    }
}
