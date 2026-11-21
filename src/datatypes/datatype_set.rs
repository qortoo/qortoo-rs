use std::{collections::BTreeMap, sync::Arc};

use crate::{
    Counter, DataType, Datatype, DatatypeState, Map, Variable,
    clients::common::ClientCommon,
    datatypes::{
        common::Attribute, datatype::DatatypeBlanket, option::DatatypeOption,
        transactional::TransactionalDatatype,
    },
    types::common::ArcStr,
};

/// A concrete datatype handle returned by client lookups and passed to handlers.
///
/// Match the variant to access the datatype's methods. Cloning this enum
/// clones the contained handle, which shares the same local value and state.
#[derive(Clone)]
pub enum DatatypeSet {
    /// A counter handle.
    Counter(Counter),
    /// A variable handle.
    Variable(Variable),
    /// A map handle.
    Map(Map),
}

impl DatatypeSet {
    /// Returns the kind of the contained datatype.
    pub fn get_type(&self) -> DataType {
        match self {
            DatatypeSet::Counter(_) => DataType::Counter,
            DatatypeSet::Variable(_) => DataType::Variable,
            DatatypeSet::Map(_) => DataType::Map,
        }
    }

    /// Returns the contained datatype's current lifecycle state.
    pub fn get_state(&self) -> DatatypeState {
        match self {
            DatatypeSet::Counter(cnt) => cnt.get_state(),
            DatatypeSet::Variable(var) => var.get_state(),
            DatatypeSet::Map(map) => map.get_state(),
        }
    }

    /// Returns the contained datatype's [logical state size in bytes](Datatype::get_state_bytes).
    pub fn get_state_bytes(&self) -> u64 {
        match self {
            Self::Counter(counter) => counter.get_state_bytes(),
            Self::Variable(variable) => variable.get_state_bytes(),
            Self::Map(map) => map.get_state_bytes(),
        }
    }

    pub(crate) fn get_core_id(&self) -> usize {
        match self {
            DatatypeSet::Counter(cnt) => cnt.get_core() as *const TransactionalDatatype as usize,
            DatatypeSet::Variable(var) => var.get_core() as *const TransactionalDatatype as usize,
            DatatypeSet::Map(map) => map.get_core() as *const TransactionalDatatype as usize,
        }
    }

    pub(crate) fn unsubscribe(&self) -> Result<(), crate::DatatypeError> {
        match self {
            DatatypeSet::Counter(cnt) => cnt.unsubscribe(),
            DatatypeSet::Variable(var) => var.unsubscribe(),
            DatatypeSet::Map(map) => map.unsubscribe(),
        }
    }

    /// Consumes this wrapper, returning the [`Counter`] it holds, or `None` if it
    /// holds a different datatype.
    pub(crate) fn into_counter(self) -> Option<Counter> {
        match self {
            DatatypeSet::Counter(cnt) => Some(cnt),
            _ => None,
        }
    }

    /// Consumes this wrapper, returning the [`Variable`] it holds, or `None` if it
    /// holds a different datatype.
    pub(crate) fn into_variable(self) -> Option<Variable> {
        match self {
            DatatypeSet::Variable(var) => Some(var),
            _ => None,
        }
    }

    pub(crate) fn into_map(self) -> Option<Map> {
        match self {
            DatatypeSet::Map(map) => Some(map),
            _ => None,
        }
    }

    /// Creates a new [`DatatypeSet`] instance for the given `type` and `key`.
    ///
    /// This is primarily used by the client internals to construct
    /// a concrete datatype variant tied to a specific client context.
    pub(crate) fn new(
        r#type: DataType,
        key: ArcStr,
        state: DatatypeState,
        client_common: Arc<ClientCommon>,
        option: DatatypeOption,
        is_readonly: bool,
        handlers: BTreeMap<usize, crate::DatatypeHandler>,
    ) -> Self {
        let attr = Arc::new(Attribute::new(
            key,
            r#type,
            client_common,
            option,
            is_readonly,
        ));
        Self::from_core(TransactionalDatatype::new_arc(attr, state, handlers))
    }

    pub(crate) fn from_core(datatype: Arc<TransactionalDatatype>) -> Self {
        match datatype.core.attr.r#type {
            DataType::Counter => DatatypeSet::Counter(Counter::new(datatype)),
            DataType::Variable => DatatypeSet::Variable(Variable::new(datatype)),
            DataType::Map => DatatypeSet::Map(Map::new(datatype)),
        }
    }
}

impl From<Counter> for DatatypeSet {
    fn from(value: Counter) -> Self {
        Self::Counter(value)
    }
}

impl From<Variable> for DatatypeSet {
    fn from(value: Variable) -> Self {
        Self::Variable(value)
    }
}

impl From<Map> for DatatypeSet {
    fn from(value: Map) -> Self {
        Self::Map(value)
    }
}

#[cfg(test)]
mod tests_datatype_set {
    use tracing::instrument;

    use crate::{
        Counter, DataType, Datatype, DatatypeState, Variable,
        clients::common::new_client_common,
        datatypes::{
            datatype::DatatypeBlanket, datatype_set::DatatypeSet,
            transactional::TransactionalDatatype,
        },
    };

    #[test]
    #[instrument]
    fn can_clone_datatype_set() {
        let ds1 = DatatypeSet::new(
            DataType::Counter,
            "k1".into(),
            DatatypeState::Creating,
            new_client_common!(),
            Default::default(),
            false,
            Default::default(),
        );
        let ds2 = ds1.clone();
        let cnt1 = ds1.into_counter().unwrap();
        let cnt2 = ds2.into_counter().unwrap();

        // Cloned DatatypeSet contains a cloned Counter (same variant, same key)
        assert_eq!(cnt1.get_key(), cnt2.get_key());
        assert_eq!(cnt1.get_type(), cnt2.get_type());

        // Verify the cloned Counter operates correctly and shares state
        assert_eq!(0, cnt1.get_value());
        assert_eq!(2, cnt2.increase_by(2).unwrap());
        assert_eq!(2, cnt1.get_value());

        // Verify the cloned Counter is different from the original
        let ptr1: *const Counter = &cnt1;
        let ptr2: *const Counter = &cnt2;
        assert_ne!(ptr1, ptr2);

        // Verify the cloned Counter has the same TransactionalDatatype as the original
        let ptr1: *const TransactionalDatatype = cnt1.get_core();
        let ptr2: *const TransactionalDatatype = cnt2.get_core();
        assert_eq!(ptr1, ptr2);
    }

    #[test]
    #[instrument]
    fn can_verify_from_into() {
        let counter = Counter::new_for_test(Default::default());
        fn assert_datatype_set(_ds: DatatypeSet) {}
        assert_datatype_set(counter.into());
    }

    #[test]
    #[instrument]
    fn can_construct_a_variable_datatype_set() {
        let ds = DatatypeSet::new(
            DataType::Variable,
            "k1".into(),
            DatatypeState::Creating,
            new_client_common!(),
            Default::default(),
            false,
            Default::default(),
        );
        assert_eq!(ds.get_type(), DataType::Variable);
        assert_eq!(ds.get_state(), DatatypeState::Creating);
        assert!(ds.into_variable().is_some());
    }

    #[test]
    #[instrument]
    fn can_reject_the_mismatched_accessor_for_each_variant() {
        let counter_ds = DatatypeSet::from(Counter::new_for_test(Default::default()));
        assert!(counter_ds.into_variable().is_none());

        let variable_ds = DatatypeSet::from(Variable::new_for_test(Default::default()));
        assert!(variable_ds.into_counter().is_none());
    }
}
