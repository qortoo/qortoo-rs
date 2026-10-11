use std::{collections::BTreeMap, sync::Arc};

use parking_lot::{Mutex, MutexGuard, RwLock};

use crate::{
    DatatypeHandler, DatatypeState,
    datatypes::{common::Attribute, effects::DatatypeEffectBatch, mutable::MutableDatatype},
};

/// State and synchronization shared by the local and wired datatype layers.
/// When multiple locks are needed, acquire tx_mutex, effects_mutex, then mutable.
/// Release mutable state before dispatching effects, keeping effects_mutex held.
pub(crate) struct DatatypeCore {
    pub(crate) attr: Arc<Attribute>,
    pub(crate) mutable: RwLock<MutableDatatype>,
    pub(crate) tx_mutex: Mutex<()>,
    effects_mutex: Mutex<()>,
}

impl DatatypeCore {
    /// Mutates state and extracts its batch, releasing mutable state before returning.
    /// The returned guard keeps the effects gate held through dispatch and follow-up work.
    /// Callers acquire the transaction gate first when their operation needs it.
    #[must_use = "Dispatch the prepared effects and retain the guard through follow-up work"]
    pub(crate) fn prepare_state_effects<R>(
        &self,
        mutate: impl FnOnce(&mut MutableDatatype) -> R,
    ) -> (R, StateEffectsGuard<'_>) {
        let gate = self.effects_mutex.lock();
        let (result, batch) = {
            let mut mutable = self.mutable.write();
            let result = mutate(&mut mutable);
            (result, mutable.take_effects())
        };
        (result, StateEffectsGuard { _gate: gate, batch })
    }

    pub(crate) fn new_arc(
        attr: Arc<Attribute>,
        state: DatatypeState,
        handlers: BTreeMap<usize, DatatypeHandler>,
    ) -> Arc<Self> {
        Arc::new(Self {
            mutable: RwLock::new(MutableDatatype::new(attr.clone(), state, handlers)),
            attr,
            tx_mutex: Mutex::new(()),
            effects_mutex: Mutex::new(()),
        })
    }
}

/// Holds the effects gate until the caller finishes dispatch and follow-up work.
/// Dropping the guard releases the gate without dispatching pending effects.
#[must_use]
pub(crate) struct StateEffectsGuard<'a> {
    _gate: MutexGuard<'a, ()>,
    batch: DatatypeEffectBatch,
}

impl StateEffectsGuard<'_> {
    /// Consumes the prepared batch while retaining the effects gate.
    pub(crate) fn dispatch(&mut self) {
        std::mem::take(&mut self.batch).dispatch();
    }
}
