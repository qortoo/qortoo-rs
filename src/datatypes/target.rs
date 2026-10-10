use std::sync::{Arc, Weak};

use crate::{
    Counter, DataType, Variable,
    datatypes::{datatype_set::DatatypeSet, transactional::TransactionalDatatype},
};

/// Captures an instance without extending its lifetime or consulting the client table.
pub(crate) struct DatatypeTarget {
    transactional: Weak<TransactionalDatatype>,
}

impl DatatypeTarget {
    pub(crate) fn new(transactional: Weak<TransactionalDatatype>) -> Self {
        Self { transactional }
    }

    pub(crate) fn resolve(&self) -> Option<DatatypeSet> {
        let transactional = self.transactional.upgrade()?;
        match transactional.core.attr.r#type {
            DataType::Counter => Some(DatatypeSet::Counter(Counter::new(transactional))),
            DataType::Variable => Some(DatatypeSet::Variable(Variable::new(transactional))),
            DataType::Map => None,
        }
    }

    pub(crate) fn detach(&self) {
        if let Some(transactional) = self.transactional.upgrade() {
            let attr = &transactional.core.attr;
            attr.client_common
                .detach_datatype_if_same_instance(&attr.key, Arc::as_ptr(&transactional) as usize);
        }
    }
}
