use std::{
    collections::{HashMap, VecDeque},
    fmt::Display,
    sync::{Arc, Weak},
};

use crossbeam_channel::Sender;
use tracing::{instrument, trace};

use crate::{
    DataType, DatatypeState,
    datatypes::{Crdt, common::Attribute, event_loop::Event, wired::WiredDatatype},
    errors::{connectivity::ConnectivityError, messages::sync as msg, push_pull::PushPullError},
    operations::{Operation, transaction::Transaction},
    types::{
        checkpoint::CheckPoint,
        common::ArcStr,
        notification::Notification,
        operation_context::OperationContext,
        push_pull_pack::PushPullPack,
        uid::{Cuid, Duid},
    },
};

macro_rules! datatype_server_instrument {
    ($(#[$attr:meta])* $vis:vis fn $name:ident $($rest:tt)*) => {
        $(#[$attr])*
        #[tracing::instrument(skip_all,
            fields(
                collection=%self.collection,
                data_key=%self.key,
                duid=%self.duid,
                r#type=%self.r#type,
                sseq=%self.synced.sseq_at_server,
            )
        )]
        $vis fn $name $($rest)*
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Subscription {
    pub(super) sseq_at_client: u64,
    pub(super) cseq_at_server: u64,
    // Includes the bootstrap floor, not just the client's reported acknowledgement.
    pub(super) sseq_gc_for_client: u64,
}

impl Subscription {
    pub(super) fn new(sseq_at_client: u64, cseq_at_server: u64, sseq_gc_for_client: u64) -> Self {
        Self {
            sseq_at_client,
            cseq_at_server,
            sseq_gc_for_client,
        }
    }

    #[cfg(test)]
    pub(super) fn checkpoint(&self) -> CheckPoint {
        CheckPoint::new(self.sseq_at_client, self.cseq_at_server)
    }
}

/// Backend-synced state; clients may not have applied this server boundary yet.
struct SyncedState {
    crdt: Crdt,
    lamport: u64,
    sseq_at_server: u64,
    // Active subscriptions: accepted client cseq and reported applied server sseq.
    checkpoints: HashMap<Cuid, Subscription>,
    history: VecDeque<Arc<Transaction>>,
    sseq_history_pruned: u64,
    sseq_gc_safe: u64,
}

impl SyncedState {
    fn new(r#type: DataType) -> Self {
        Self {
            crdt: Crdt::new(r#type),
            lamport: 0,
            sseq_at_server: 0,
            checkpoints: HashMap::new(),
            history: VecDeque::new(),
            sseq_history_pruned: 0,
            sseq_gc_safe: 0,
        }
    }

    fn has_active_subscription(&self, cuid: &Cuid) -> bool {
        self.checkpoints.contains_key(cuid)
    }

    fn is_ahead_of_server(&self, sseq: u64) -> bool {
        sseq > self.sseq_at_server
    }

    fn requires_pruned_history(&self, sseq: u64) -> bool {
        sseq < self.sseq_history_pruned
    }

    fn cseq_at_server(&self, cuid: &Cuid) -> u64 {
        self.checkpoints
            .get(cuid)
            .map_or(0, |checkpoint| checkpoint.cseq_at_server)
    }

    fn get_or_insert_checkpoint(
        &mut self,
        pushed: &PushPullPack,
        initial_sseq: u64,
    ) -> &mut Subscription {
        self.checkpoints
            .entry(pushed.cuid.clone())
            .or_insert_with(|| Subscription::new(initial_sseq, 0, self.sseq_gc_safe))
    }

    fn push_transactions(&mut self, pushed: &PushPullPack) -> Result<(u64, bool), PushPullError> {
        let previous_cseq = self.cseq_at_server(&pushed.cuid);
        let mut incoming = pushed.transactions_after_cseq(previous_cseq).peekable();
        if incoming.peek().is_none() {
            return Ok((previous_cseq, false));
        }

        // Publish state and progress only after the entire new batch applies successfully.
        let mut crdt = self.crdt.fork_for_batch();
        let mut lamport = self.lamport;
        let mut sseq = self.sseq_at_server;
        let mut cseq = previous_cseq;
        let mut transactions = Vec::new();
        for tx in incoming {
            if tx.cuid != pushed.cuid
                || cseq.checked_add(1) != Some(tx.cseq)
                || tx.operations.is_empty()
            {
                return Err(PushPullError::ProtocolViolation(
                    msg::INVALID_TRANSACTION.into(),
                ));
            }
            for operation in tx.iter() {
                let context = OperationContext::try_new(operation, &tx.cuid)
                    .map_err(|e| PushPullError::ProtocolViolation(e.to_string()))?;
                crdt.execute_remote_operation(&context)
                    .map_err(|e| PushPullError::ProtocolViolation(e.to_string()))?;
                lamport = lamport.max(operation.lamport);
            }
            sseq += 1;
            let mut owned_tx: Transaction = (**tx).clone();
            owned_tx.sseq = sseq;
            transactions.push(Arc::new(owned_tx));
            cseq = tx.cseq;
        }
        self.crdt = crdt;
        self.lamport = lamport;
        self.sseq_at_server = sseq;
        self.history.extend(transactions);
        self.get_or_insert_checkpoint(pushed, pushed.checkpoint.sseq)
            .cseq_at_server = cseq;
        Ok((cseq, true))
    }

    fn sseq_history_prunable(&self) -> u64 {
        self.checkpoints
            .values()
            .map(|cp| cp.sseq_at_client)
            .min()
            .unwrap_or(self.sseq_at_server)
    }

    fn prune_history(&mut self) {
        let sseq_history_prunable = self.sseq_history_prunable();
        while self
            .history
            .front()
            .is_some_and(|tx| tx.sseq <= sseq_history_prunable)
        {
            self.sseq_history_pruned = self.history.pop_front().unwrap().sseq;
        }
    }

    fn get_subscribe_snapshot(&self) -> Transaction {
        let mut operation = Operation::new_snapshot(self.crdt.serialize());
        operation.set_lamport(self.lamport);
        let mut transaction = Transaction::new_with_cuid(&Cuid::new_nil());
        transaction.sseq = self.sseq_at_server;
        transaction.push_operation(operation);
        transaction
    }

    fn pull_transactions(&self, pulled: &mut PushPullPack) {
        let from_sseq = pulled.checkpoint.sseq;
        for tx in &self.history {
            if tx.sseq > from_sseq && tx.cuid != pulled.cuid {
                pulled.transactions.push(tx.clone());
            }
        }
        pulled.checkpoint.sseq = self.sseq_at_server;
    }
}

pub struct LocalDatatypeServer {
    // The event loop owns each wired instance. Backend registration must not retain its runtime.
    wired_map: HashMap<Cuid, Weak<WiredDatatype>>,
    sender_map: HashMap<Cuid, Sender<Event>>,
    collection: ArcStr,
    key: ArcStr,
    r#type: DataType,
    // Nil until a creation request succeeds.
    duid: Duid,
    synced: SyncedState,
    pub(super) max_sub_gens: HashMap<Cuid, u64>,
}

impl Display for LocalDatatypeServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{} '{}' subscribed by {} clients, sseq: {} created: {}",
            self.r#type,
            self.key,
            self.wired_map.len(),
            self.synced.sseq_at_server,
            self.is_created()
        ))
    }
}

impl LocalDatatypeServer {
    pub(super) fn new(attr: &Attribute, max_sub_gens: HashMap<Cuid, u64>) -> Self {
        Self {
            max_sub_gens,
            wired_map: HashMap::new(),
            sender_map: HashMap::new(),
            synced: SyncedState::new(attr.r#type),
            collection: attr.client_common.collection.clone(),
            key: attr.key.clone(),
            r#type: attr.r#type,
            duid: Duid::new_nil(),
        }
    }

    fn is_created(&self) -> bool {
        !self.duid.is_nil()
    }

    pub fn insert_client_item(&mut self, wired: Arc<WiredDatatype>, sender: Sender<Event>) {
        self.wired_map.insert(wired.cuid(), Arc::downgrade(&wired));
        self.sender_map.insert(wired.cuid(), sender);
    }

    pub fn is_empty(&self) -> bool {
        // Both must be empty: `wired_map` can hold in-process clients registered before
        // their first sync, which are not yet in `checkpoints`.
        self.wired_map.is_empty() && self.synced.checkpoints.is_empty()
    }

    #[cfg(test)]
    pub fn remove_client_subscription(&mut self, cuid: &Cuid) {
        self.wired_map.remove(cuid);
        self.sender_map.remove(cuid);
        self.synced.checkpoints.remove(cuid);
    }

    fn validate_gc_ack(&self, pushed: &PushPullPack) -> Result<(), PushPullError> {
        if pushed.sseq_gc_safe > pushed.checkpoint.sseq {
            return Err(PushPullError::ProtocolViolation(msg::INVALID_GC_ACK.into()));
        }
        Ok(())
    }

    fn validate_progress(&self, pushed: &PushPullPack) -> Result<(), PushPullError> {
        self.validate_gc_ack(pushed)?;
        if !self.synced.has_active_subscription(&pushed.cuid) {
            return Err(PushPullError::MissingSubscription(
                msg::missing_active_subscription(&pushed.cuid),
            ));
        }
        if self.duid != pushed.duid || self.r#type != pushed.r#type {
            return Err(PushPullError::ProtocolViolation(
                msg::INVALID_IDENTITY.into(),
            ));
        }
        if self.synced.is_ahead_of_server(pushed.checkpoint.sseq) {
            return Err(PushPullError::ProtocolViolation(
                msg::INVALID_CHECKPOINT.into(),
            ));
        }
        if self.synced.requires_pruned_history(pushed.checkpoint.sseq) {
            return Err(PushPullError::ProtocolViolation(msg::HISTORY_PRUNED.into()));
        }
        Ok(())
    }

    fn validate_creation(&self, pushed: &PushPullPack) -> Result<(), PushPullError> {
        self.validate_gc_ack(pushed)?;
        if self.is_created() && self.duid != pushed.duid {
            return Err(PushPullError::CreateFailed(msg::ALREADY_EXISTS.into()));
        }
        // Existing resources require a snapshot before a new generation can write.
        if self.is_created() && !self.synced.has_active_subscription(&pushed.cuid) {
            return Err(PushPullError::MissingSubscription(
                msg::CREATION_REQUIRES_SUBSCRIPTION.into(),
            ));
        }
        if pushed.duid.is_nil() || self.r#type != pushed.r#type {
            return Err(PushPullError::ProtocolViolation(
                msg::INVALID_CREATION.into(),
            ));
        }
        if self.synced.is_ahead_of_server(pushed.checkpoint.sseq) {
            return Err(PushPullError::ProtocolViolation(
                msg::INVALID_CHECKPOINT.into(),
            ));
        }
        if self.synced.has_active_subscription(&pushed.cuid) {
            self.validate_progress(pushed)?;
        } else if self.synced.requires_pruned_history(pushed.checkpoint.sseq) {
            return Err(PushPullError::ProtocolViolation(msg::HISTORY_PRUNED.into()));
        }
        if pushed.is_readonly {
            return Err(PushPullError::ReadonlyViolation);
        }
        Ok(())
    }

    fn validate_subscription(&self, pushed: &PushPullPack) -> Result<(), PushPullError> {
        self.validate_gc_ack(pushed)?;
        if !self.is_created() {
            return Err(PushPullError::ResourceNotFound(msg::resource_not_found(
                pushed.r#type,
                pushed.resource_id(),
            )));
        }
        if self.r#type != pushed.r#type {
            return Err(PushPullError::ResourceNotFound(
                msg::resource_type_mismatch(pushed.resource_id(), pushed.r#type, self.r#type),
            ));
        }
        if !pushed.transactions.is_empty() {
            return Err(PushPullError::ProtocolViolation(
                msg::SUBSCRIBING_PUSH.into(),
            ));
        }

        if self.synced.is_ahead_of_server(pushed.checkpoint.sseq) {
            return Err(PushPullError::ProtocolViolation(
                msg::INVALID_CHECKPOINT.into(),
            ));
        }
        if !self.synced.has_active_subscription(&pushed.cuid) && pushed.checkpoint.sseq != 0 {
            return Err(PushPullError::ProtocolViolation(
                msg::INVALID_SUBSCRIPTION_CHECKPOINT.into(),
            ));
        }
        Ok(())
    }

    fn validate_client_push(&self, pushed: &PushPullPack) -> Result<(), PushPullError> {
        self.validate_progress(pushed)?;

        if pushed.is_readonly && !pushed.transactions.is_empty() {
            return Err(PushPullError::ReadonlyViolation);
        }
        Ok(())
    }

    fn update_subscription_progress(
        &mut self,
        pushed: &PushPullPack,
        pulled: &mut PushPullPack,
        initial: u64,
    ) {
        // Only a new subscription or transaction-free creation needs insertion; neither has accepted client transactions.
        let checkpoint = self.synced.get_or_insert_checkpoint(pushed, initial);
        // A bootstrap push has not applied the snapshot that this pull will provide.
        if pulled.snapshot_transaction.is_none() {
            checkpoint.sseq_at_client = checkpoint.sseq_at_client.max(pushed.checkpoint.sseq);
            checkpoint.sseq_gc_for_client = checkpoint.sseq_gc_for_client.max(pushed.sseq_gc_safe);
        }
    }

    /// Returns an immediate response when the subscription generation cannot be processed.
    fn pre_process(&self, pushed: &PushPullPack, max_sub_gen: Option<u64>) -> Option<PushPullPack> {
        let active = self.synced.has_active_subscription(&pushed.cuid);
        if !active && max_sub_gen.is_some_and(|generation| pushed.sub_gen <= generation) {
            let mut pulled = pushed.get_pulled_stub();
            pulled.sseq_gc_safe = self.synced.sseq_gc_safe;
            return Some(if pushed.state == DatatypeState::Unsubscribing {
                pulled.state = DatatypeState::Disabled;
                pulled
            } else {
                pulled.with_error(PushPullError::MissingSubscription(
                    msg::RETIRED_SUBSCRIPTION.into(),
                ))
            });
        }
        let starts_subscription = matches!(
            pushed.state,
            DatatypeState::Creating
                | DatatypeState::Subscribing
                | DatatypeState::SubscribingOrCreating
        );
        if active
            && max_sub_gen.is_some_and(|generation| {
                pushed.sub_gen < generation || (pushed.sub_gen > generation && !starts_subscription)
            })
        {
            return Some(
                pushed
                    .get_pulled_stub()
                    .with_error(PushPullError::MissingSubscription(
                        msg::SUBSCRIPTION_GENERATION_MISMATCH.into(),
                    )),
            );
        }
        None
    }

    /// Dispatches one pushed pack to the state-specific processor.
    pub fn process(
        &mut self,
        pushed: &PushPullPack,
        is_realtime: bool,
    ) -> Result<PushPullPack, ConnectivityError> {
        let max_sub_gen = self.max_sub_gens.get(&pushed.cuid).copied();
        if let Some(pulled) = self.pre_process(pushed, max_sub_gen) {
            return Ok(pulled);
        }
        // Stage a fresh generation; failed validation or batch application must retain the old progress.
        let previous = if max_sub_gen.is_some_and(|generation| pushed.sub_gen > generation) {
            self.synced.checkpoints.remove(&pushed.cuid)
        } else {
            None
        };
        let mut pulled = pushed.get_pulled_stub();
        let result = match pushed.state {
            DatatypeState::Creating => self.process_creating(pushed, &mut pulled),
            DatatypeState::Subscribing => self.process_subscribing(pushed, &mut pulled),
            DatatypeState::SubscribingOrCreating => {
                self.process_subscribing_or_creating(pushed, &mut pulled)
            }
            DatatypeState::Subscribed => self.process_subscribed(pushed, &mut pulled, is_realtime),
            DatatypeState::Unsubscribing => {
                self.process_unsubscribing(pushed, &mut pulled, is_realtime)
            }
            DatatypeState::Deleting => self.process_deleting(),
            DatatypeState::Disabled => self.process_disabled(),
        };
        Ok(self.post_process(pushed, pulled, result, previous, max_sub_gen))
    }

    fn post_process(
        &mut self,
        pushed: &PushPullPack,
        mut pulled: PushPullPack,
        result: Result<(), PushPullError>,
        previous: Option<Subscription>,
        max_sub_gen: Option<u64>,
    ) -> PushPullPack {
        // Unsubscribe closes its generation even when the final batch is rejected.
        if (result.is_ok() || pushed.state == DatatypeState::Unsubscribing)
            && max_sub_gen.is_none_or(|generation| pushed.sub_gen > generation)
        {
            self.max_sub_gens
                .insert(pushed.cuid.clone(), pushed.sub_gen);
        }
        if let Err(error) = result {
            if let Some(previous) = previous {
                self.synced
                    .checkpoints
                    .insert(pushed.cuid.clone(), previous);
            }
            pulled = pulled.with_error(error);
        } else {
            // Retain response Arcs before removing history needed by a delayed request.
            self.synced.prune_history();
            // With no protected subscribers, keep the last issued GC boundary.
            if let Some(boundary) = self
                .synced
                .checkpoints
                .values()
                .map(|cp| cp.sseq_gc_for_client)
                .min()
            {
                self.synced.sseq_gc_safe = boundary;
            }
        }
        pulled.sseq_gc_safe = self.synced.sseq_gc_safe;
        pulled
    }

    datatype_server_instrument! {
    fn process_creating(
        &mut self,
        pushed: &PushPullPack,
        pulled: &mut PushPullPack,
    ) -> Result<(), PushPullError> {
        self.validate_creation(pushed)?;
        let (cseq, _) = self.synced.push_transactions(pushed)?;
        self.duid = pushed.duid.clone();
        pulled.checkpoint = CheckPoint::new(pushed.checkpoint.sseq, cseq);
        // A creation retry after a lost response must also pull intervening changes from other clients.
        self.synced.pull_transactions(pulled);
        pulled.state = DatatypeState::Subscribed;
        self.update_subscription_progress(pushed, pulled, 0);
        Ok(())
    }}

    datatype_server_instrument! {
    fn process_subscribing_or_creating(
        &mut self,
        pushed: &PushPullPack,
        pulled: &mut PushPullPack,
    ) -> Result<(), PushPullError> {
        if self.is_created() && self.duid != pushed.duid {
            self.process_subscribing(pushed, pulled)
        } else {
            self.process_creating(pushed, pulled)
        }
    }}

    datatype_server_instrument! {
    fn process_subscribed(
        &mut self,
        pushed: &PushPullPack,
        pulled: &mut PushPullPack,
        is_realtime: bool,
    ) -> Result<(), PushPullError> {
        self.process_push_pull(pushed, pulled, DatatypeState::Subscribed, is_realtime)
    }}

    fn process_push_pull(
        &mut self,
        pushed: &PushPullPack,
        pulled: &mut PushPullPack,
        success_state: DatatypeState,
        is_realtime: bool,
    ) -> Result<(), PushPullError> {
        self.validate_client_push(pushed)?;
        let (cseq, pushed_any) = self.synced.push_transactions(pushed)?;
        pulled.checkpoint = CheckPoint::new(pushed.checkpoint.sseq, cseq);
        self.synced.pull_transactions(pulled);
        pulled.state = success_state;
        self.update_subscription_progress(pushed, pulled, 0);
        if is_realtime && pushed_any {
            self.notify_pushed(&pushed.cuid);
        }

        Ok(())
    }

    #[instrument(skip_all)]
    fn notify_pushed(&self, cuid: &Cuid) {
        let notification = Notification::new(
            cuid.clone(),
            self.duid.clone(),
            self.synced.sseq_at_server,
            0,
        );
        let mut notified_cuids = Vec::new();
        for (registered_cuid, sender) in &self.sender_map {
            if registered_cuid == cuid {
                continue;
            }
            match sender.try_send(Event::Notify(notification.clone())) {
                Ok(_) => notified_cuids.push(registered_cuid.to_string()),
                Err(e) => trace!("failed to notify {registered_cuid}: {e:?}"),
            }
        }
        trace!(
            "notified {} client(s): {notified_cuids:?} of {notification}",
            notified_cuids.len()
        );
    }

    datatype_server_instrument! {
    fn process_unsubscribing(
        &mut self,
        pushed: &PushPullPack,
        pulled: &mut PushPullPack,
        is_realtime: bool,
    ) -> Result<(), PushPullError> {
        let result = if self.synced.has_active_subscription(&pushed.cuid) {
            self.process_push_pull(pushed, pulled, DatatypeState::Disabled, is_realtime)
        } else {
            pulled.state = DatatypeState::Disabled;
            Ok(())
        };

        // Always clean up client subscription regardless of error: the client will be Disabled
        // either way, and leaving stale entries would cause infinite unsubscribe retry loops.
        self.wired_map.remove(&pushed.cuid);
        self.sender_map.remove(&pushed.cuid);
        self.synced.checkpoints.remove(&pushed.cuid);

        result
    }}

    datatype_server_instrument! {
    fn process_deleting(&mut self) -> Result<(), PushPullError> {
        Err(PushPullError::ProtocolViolation(
            msg::DELETING_UNSUPPORTED.into(),
        ))
    }}

    datatype_server_instrument! {
    fn process_disabled(&mut self) -> Result<(), PushPullError> {
        Err(PushPullError::ProtocolViolation(
            msg::DISABLED_PUSH.into(),
        ))
    }}

    datatype_server_instrument! {
    fn process_subscribing(
        &mut self,
        pushed: &PushPullPack,
        pulled: &mut PushPullPack,
    ) -> Result<(), PushPullError> {
        self.validate_subscription(pushed)?;
        let initial = self.synced.sseq_history_prunable();
        pulled.duid = self.duid.clone();
        let snapshot = self.synced.get_subscribe_snapshot();
        pulled.checkpoint = CheckPoint::new(
            snapshot.sseq,
            self.synced.cseq_at_server(&pushed.cuid),
        );
        pulled.snapshot_transaction = Some(Arc::new(snapshot));
        pulled.state = DatatypeState::Subscribed;
        self.update_subscription_progress(pushed, pulled, initial);
        Ok(())
    }}

    #[cfg(test)]
    pub fn get_wired_datatype(&self, cuid: &Cuid) -> Option<Arc<WiredDatatype>> {
        self.wired_map.get(cuid).and_then(Weak::upgrade)
    }
}

#[cfg(test)]
mod tests;
