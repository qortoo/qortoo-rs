use std::sync::{Arc, Weak};

use crate::datatypes::{datatype_set::DatatypeSet, transactional::TransactionalDatatype};

/// Captures an instance without extending its lifetime or consulting the client table.
pub(crate) struct DatatypeTarget {
    transactional: Weak<TransactionalDatatype>,
}

impl DatatypeTarget {
    pub(crate) fn new(transactional: Weak<TransactionalDatatype>) -> Self {
        Self { transactional }
    }

    pub(crate) fn resolve(&self) -> Option<DatatypeSet> {
        self.transactional.upgrade().map(DatatypeSet::from_core)
    }

    pub(crate) fn detach(&self) {
        if let Some(transactional) = self.transactional.upgrade() {
            let attr = &transactional.core.attr;
            attr.client_common
                .detach_datatype_if_same_instance(&attr.key, Arc::as_ptr(&transactional) as usize);
        }
    }
}
