use std::{
    collections::HashMap,
    fmt::Display,
    sync::{Arc, Weak},
};

use crossbeam_channel::Sender;
use tracing::{instrument, trace};

use crate::{
    DataType, DatatypeState,
    datatypes::{Crdt, common::Attribute, event_loop::Event, wired::WiredDatatype},
    errors::{connectivity::ConnectivityError, push_pull::PushPullError},
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

const ERR_INVALID_TRANSACTION: &str = "invalid transaction identity or client sequence";
const ERR_INVALID_IDENTITY: &str = "invalid datatype identity";
const ERR_INVALID_CHECKPOINT: &str = "checkpoint exceeds the server head";
const ERR_ALREADY_EXISTS: &str = "already exist";
const ERR_INVALID_CREATION: &str = "invalid creation DUID or type";
const ERR_DISABLED_PUSH: &str = "disabled client attempted push";
const ERR_SUBSCRIBING_PUSH: &str = "cannot push transactions when subscribing";
const ERR_INVALID_SUBSCRIPTION_CHECKPOINT: &str = "invalid subscription checkpoint";

macro_rules! datatype_server_instrument {
    ($(#[$attr:meta])* $vis:vis fn $name:ident $($rest:tt)*) => {
        $(#[$attr])*
        #[tracing::instrument(skip_all,
            fields(
                collection=%self.collection,
                data_key=%self.key,
                duid=%self.duid,
                r#type=%self.r#type,
                sseq=%self.synced.sseq,
            )
        )]
        $vis fn $name $($rest)*
    };
}

/// Backend-synced state; clients may not have applied this server boundary yet.
struct SyncedState {
    crdt: Crdt,
    lamport: u64,
    sseq: u64,
    // Active subscriptions: accepted client cseq and reported applied server sseq.
    checkpoints: HashMap<Cuid, CheckPoint>,
    history: Vec<Arc<Transaction>>,
}

impl SyncedState {
    fn new(r#type: DataType) -> Self {
        Self {
            crdt: Crdt::new(r#type),
            lamport: 0,
            sseq: 0,
            checkpoints: HashMap::new(),
            history: Vec::new(),
        }
    }

    fn push_transactions(&mut self, pushed: &PushPullPack) -> Result<(u64, bool), PushPullError> {
        let checkpoint = self
            .checkpoints
            .get(&pushed.cuid)
            .copied()
            .unwrap_or_else(|| CheckPoint::new(self.safe_sseq(), 0));
        let previous_cseq = checkpoint.cseq;
        let mut incoming = pushed.transactions_after_cseq(previous_cseq).peekable();
        if incoming.peek().is_none() {
            return Ok((previous_cseq, false));
        }

        // Publish state and progress only after the entire new batch applies successfully.
        let mut crdt = self.crdt.fork_for_batch();
        let mut lamport = self.lamport;
        let mut sseq = self.sseq;
        let mut cseq = previous_cseq;
        let mut transactions = Vec::new();
        for tx in incoming {
            if tx.cuid != pushed.cuid
                || cseq.checked_add(1) != Some(tx.cseq)
                || tx.operations.is_empty()
            {
                return Err(PushPullError::ProtocolViolation(
                    ERR_INVALID_TRANSACTION.into(),
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
        self.sseq = sseq;
        self.history.extend(transactions);
        self.checkpoints
            .entry(pushed.cuid.clone())
            .or_insert(checkpoint)
            .cseq = cseq;
        Ok((cseq, true))
    }

    fn safe_sseq(&self) -> u64 {
        self.checkpoints
            .values()
            .map(|cp| cp.sseq)
            .min()
            .unwrap_or(self.sseq)
    }

    fn get_subscribe_snapshot(&self) -> Transaction {
        let mut operation = Operation::new_snapshot(self.crdt.serialize());
        operation.set_lamport(self.lamport);
        let mut transaction = Transaction::new_with_cuid(&Cuid::new_nil());
        transaction.sseq = self.sseq;
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
        pulled.checkpoint.sseq = self.sseq;
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
}

impl Display for LocalDatatypeServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{} '{}' subscribed by {} clients, sseq: {} created: {}",
            self.r#type,
            self.key,
            self.wired_map.len(),
            self.synced.sseq,
            self.is_created()
        ))
    }
}

impl LocalDatatypeServer {
    pub fn new(attr: &Attribute) -> Self {
        Self {
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

    fn validate_progress(&self, pushed: &PushPullPack) -> Result<(), PushPullError> {
        if !self.synced.checkpoints.contains_key(&pushed.cuid) {
            return Err(PushPullError::MissingSubscription(format!(
                "cuid '{}' has no active datatype subscription on this server",
                pushed.cuid
            )));
        }
        if self.duid != pushed.duid || self.r#type != pushed.r#type {
            return Err(PushPullError::ProtocolViolation(
                ERR_INVALID_IDENTITY.into(),
            ));
        }
        if pushed.checkpoint.sseq > self.synced.sseq {
            return Err(PushPullError::ProtocolViolation(
                ERR_INVALID_CHECKPOINT.into(),
            ));
        }
        Ok(())
    }

    fn validate_creation(&self, pushed: &PushPullPack) -> Result<(), PushPullError> {
        // If already created, an error should occur,
        // but if the DUID is the same, it is considered a duplicate transmission case and is allowed.
        if self.is_created() && self.duid != pushed.duid {
            return Err(PushPullError::CreateFailed(ERR_ALREADY_EXISTS.into()));
        }
        if pushed.duid.is_nil() || self.r#type != pushed.r#type {
            return Err(PushPullError::ProtocolViolation(
                ERR_INVALID_CREATION.into(),
            ));
        }
        if pushed.checkpoint.sseq > self.synced.sseq {
            return Err(PushPullError::ProtocolViolation(
                ERR_INVALID_CHECKPOINT.into(),
            ));
        }
        if self.is_created() {
            self.validate_progress(pushed)?;
        }
        if pushed.is_readonly {
            return Err(PushPullError::ReadonlyViolation);
        }
        Ok(())
    }

    fn validate_subscription(&self, pushed: &PushPullPack) -> Result<(), PushPullError> {
        if !self.is_created() {
            return Err(PushPullError::ResourceNotFound(format!(
                "{} '{}' not exists",
                pushed.r#type,
                pushed.resource_id(),
            )));
        }
        if self.r#type != pushed.r#type {
            return Err(PushPullError::ResourceNotFound(format!(
                "mismatched types for '{}': pushed type-{} but existed type {}",
                pushed.resource_id(),
                pushed.r#type,
                self.r#type,
            )));
        }
        if !pushed.transactions.is_empty() {
            return Err(PushPullError::ProtocolViolation(
                ERR_SUBSCRIBING_PUSH.into(),
            ));
        }

        if pushed.checkpoint.sseq > self.synced.sseq {
            return Err(PushPullError::ProtocolViolation(
                ERR_INVALID_CHECKPOINT.into(),
            ));
        }
        if !self.synced.checkpoints.contains_key(&pushed.cuid) && pushed.checkpoint.sseq != 0 {
            return Err(PushPullError::ProtocolViolation(
                ERR_INVALID_SUBSCRIPTION_CHECKPOINT.into(),
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
        let checkpoint = self
            .synced
            .checkpoints
            .entry(pushed.cuid.clone())
            .or_insert(CheckPoint::new(initial, 0));
        // A bootstrap push has not applied the snapshot that this pull will provide.
        if pulled.snapshot_transaction.is_none() {
            checkpoint.sseq = checkpoint.sseq.max(pushed.checkpoint.sseq);
        }
    }

    /// Dispatches one pushed pack to the state-specific processor.
    pub fn process(
        &mut self,
        pushed: &PushPullPack,
        is_realtime: bool,
    ) -> Result<PushPullPack, ConnectivityError> {
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
        if let Err(error) = result {
            pulled = pulled.with_error(error);
        }
        pulled.safe_sseq = self.synced.safe_sseq();
        Ok(pulled)
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
        let notification = Notification::new(cuid.clone(), self.duid.clone(), self.synced.sseq, 0);
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
        // If the client's datatype is not subscribed on this server, skip push processing to avoid
        // polluting checkpoints, and return Disabled directly since that is the desired state.
        if !self.synced.checkpoints.contains_key(&pushed.cuid) {
            pulled.state = DatatypeState::Disabled;
            self.wired_map.remove(&pushed.cuid);
            self.sender_map.remove(&pushed.cuid);
            return Ok(());
        }

        let result = self.process_push_pull(pushed, pulled, DatatypeState::Disabled, is_realtime);

        // Always clean up client subscription regardless of error: the client will be Disabled
        // either way, and leaving stale entries would cause infinite unsubscribe retry loops.
        self.wired_map.remove(&pushed.cuid);
        self.sender_map.remove(&pushed.cuid);
        self.synced.checkpoints.remove(&pushed.cuid);

        result
    }}

    datatype_server_instrument! {
    fn process_deleting(&mut self) -> Result<(), PushPullError> {
        Ok(())
    }}

    datatype_server_instrument! {
    fn process_disabled(&mut self) -> Result<(), PushPullError> {
        Err(PushPullError::ProtocolViolation(
            ERR_DISABLED_PUSH.into(),
        ))
    }}

    datatype_server_instrument! {
    fn process_subscribing(
        &mut self,
        pushed: &PushPullPack,
        pulled: &mut PushPullPack,
    ) -> Result<(), PushPullError> {
        self.validate_subscription(pushed)?;
        let initial = self.synced.safe_sseq();
        pulled.duid = self.duid.clone();
        let snapshot = self.synced.get_subscribe_snapshot();
        pulled.checkpoint.sseq = snapshot.sseq;
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
mod tests_local_datatype_server {
    use std::{
        sync::{Arc, mpsc},
        thread,
        time::Duration,
    };

    use rstest::rstest;
    use tracing::{info, instrument};

    use super::LocalDatatypeServer;
    use crate::{
        Client, Counter, DataType, Datatype, DatatypeError, DatatypeState,
        connectivity::local_connectivity::LocalConnectivity,
        datatypes::common::new_attribute,
        errors::{connectivity::ConnectivityError, push_pull::PushPullError},
        operations::{Operation, transaction::Transaction},
        types::{checkpoint::CheckPoint, push_pull_pack::PushPullPack, uid::Duid},
        utils::test_utils::{get_test_collection_name, get_test_func_name, get_test_ids},
    };

    fn push_no_change(_push: &mut PushPullPack) {}

    #[test]
    fn can_track_readonly_acknowledgements_without_advancing_on_snapshot_delivery() {
        let attr = new_attribute!(DataType::Counter);
        let mut server = LocalDatatypeServer::new(&attr);
        let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
        writer.transactions = vec![transaction(
            &writer.cuid,
            1,
            Operation::new_counter_increase(7),
        )];
        writer.safe_sseq = u64::MAX;
        let created = server.process(&writer, false).unwrap();
        assert_eq!(created.safe_sseq, 0);
        assert_eq!(created.checkpoint.sseq, 1);
        writer.state = DatatypeState::Subscribed;
        writer.transactions.clear();
        writer.checkpoint.sseq = 1;
        assert_eq!(server.process(&writer, false).unwrap().safe_sseq, 1);

        let mut reader = PushPullPack::new(&attr, DatatypeState::Subscribing);
        reader.cuid = crate::types::uid::Cuid::new();
        reader.is_readonly = true;
        let snapshot = server.process(&reader, false).unwrap();
        assert_eq!(snapshot.safe_sseq, 1);
        assert_eq!(server.synced.checkpoints[&reader.cuid].sseq, 1);
        writer.transactions = vec![transaction(
            &writer.cuid,
            2,
            Operation::new_counter_increase(3),
        )];
        assert_eq!(server.process(&writer, false).unwrap().safe_sseq, 1);
        writer.transactions.clear();
        writer.checkpoint.sseq = 2;
        assert_eq!(server.process(&writer, false).unwrap().safe_sseq, 1);

        // A repeated subscribe may provide a newer snapshot, but is not its acknowledgement.
        let repeated = server.process(&reader, false).unwrap();
        assert_eq!(repeated.checkpoint.sseq, 2);
        assert_eq!(repeated.safe_sseq, 1);
        reader.state = DatatypeState::Subscribed;
        reader.checkpoint.sseq = 1;
        let replay = server.process(&reader, false).unwrap();
        assert_eq!(replay.transactions.len(), 1);
        assert_eq!(replay.safe_sseq, 1);
        reader.checkpoint.sseq = 2;
        assert_eq!(server.process(&reader, false).unwrap().safe_sseq, 2);
        reader.checkpoint.sseq = 1;
        let delayed = server.process(&reader, false).unwrap();
        assert!(delayed.error.is_none());
        assert_eq!(delayed.transactions.len(), 1);
        assert_eq!(delayed.safe_sseq, 2);
        assert_eq!(server.synced.checkpoints[&reader.cuid].sseq, 2);

        reader.state = DatatypeState::Unsubscribing;
        assert_eq!(server.process(&reader, false).unwrap().safe_sseq, 2);
        assert!(!server.synced.checkpoints.contains_key(&reader.cuid));
        writer.state = DatatypeState::Unsubscribing;
        assert_eq!(server.process(&writer, false).unwrap().safe_sseq, 2);
        assert!(server.synced.checkpoints.is_empty());
    }

    #[rstest]
    #[case::future_sequence(0)]
    #[case::wrong_datatype(1)]
    #[case::missing_subscription(2)]
    #[case::failed_batch(3)]
    fn can_reject_invalid_pushes_without_advancing_applied_progress(#[case] invalid: u8) {
        let attr = new_attribute!(DataType::Counter);
        let mut server = LocalDatatypeServer::new(&attr);
        let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
        writer.transactions = vec![transaction(
            &writer.cuid,
            1,
            Operation::new_counter_increase(7),
        )];
        assert!(server.process(&writer, false).unwrap().error.is_none());
        let mut peer = PushPullPack::new(&attr, DatatypeState::Subscribing);
        peer.cuid = crate::types::uid::Cuid::new();
        server.process(&peer, false).unwrap();
        peer.state = DatatypeState::Subscribed;
        peer.transactions = vec![transaction(
            &peer.cuid,
            1,
            Operation::new_counter_increase(3),
        )];
        server.process(&peer, false).unwrap();
        assert_eq!(server.synced.sseq, 2);

        writer.state = DatatypeState::Subscribed;
        writer.checkpoint.sseq = 1;
        writer.transactions = vec![transaction(
            &writer.cuid,
            2,
            Operation::new_counter_increase(5),
        )];
        match invalid {
            // This request must be checked against the head before accepting its new transaction.
            0 => writer.checkpoint.sseq = 3,
            1 => writer.duid = Duid::new(),
            2 => writer.cuid = crate::types::uid::Cuid::new(),
            3 => {
                writer.transactions = vec![transaction(
                    &writer.cuid,
                    3,
                    Operation::new_counter_increase(5),
                )]
            }
            _ => unreachable!(),
        }
        writer.safe_sseq = u64::MAX;
        let rejected = server.process(&writer, false).unwrap();
        assert!(rejected.error.is_some());
        assert_eq!(rejected.safe_sseq, 0);
        assert_eq!(server.synced.sseq, 2);
        assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 10);
        assert_eq!(server.synced.checkpoints[&attr.get_cuid()].sseq, 0);
        assert_eq!(server.synced.checkpoints[&attr.get_cuid()].cseq, 1);
    }

    #[rstest]
    #[case::creation(DatatypeState::Creating, false)]
    #[case::subscribe_or_create(DatatypeState::SubscribingOrCreating, false)]
    #[case::creation_retry(DatatypeState::Creating, true)]
    #[case::subscribe_or_create_retry(DatatypeState::SubscribingOrCreating, true)]
    #[case::subscribed(DatatypeState::Subscribed, true)]
    #[case::subscription_retry(DatatypeState::Subscribing, true)]
    #[case::unsubscribing(DatatypeState::Unsubscribing, true)]
    fn can_report_a_consistent_error_for_a_checkpoint_beyond_the_server_head(
        #[case] state: DatatypeState,
        #[case] created: bool,
    ) {
        let attr = new_attribute!(DataType::Counter);
        let mut server = LocalDatatypeServer::new(&attr);
        let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
        if created {
            assert!(server.process(&pushed, false).unwrap().error.is_none());
        }
        pushed.state = state;
        pushed.checkpoint.sseq = 1;
        let pulled = server.process(&pushed, false).unwrap();
        assert_eq!(
            pulled.error,
            Some(PushPullError::ProtocolViolation(
                "checkpoint exceeds the server head".into()
            ))
        );
        assert_eq!(pulled.state, DatatypeState::Disabled);
        assert_eq!(server.synced.sseq, 0);
    }

    #[test]
    fn can_acknowledge_an_existing_boundary_without_a_prior_response() {
        let attr = new_attribute!(DataType::Counter);
        let mut server = LocalDatatypeServer::new(&attr);
        let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
        writer.transactions = vec![transaction(
            &writer.cuid,
            1,
            Operation::new_counter_increase(7),
        )];
        server.process(&writer, false).unwrap();
        let mut peer = PushPullPack::new(&attr, DatatypeState::Subscribing);
        peer.cuid = crate::types::uid::Cuid::new();
        server.process(&peer, false).unwrap();
        peer.state = DatatypeState::Subscribed;
        peer.transactions = vec![transaction(
            &peer.cuid,
            1,
            Operation::new_counter_increase(3),
        )];
        server.process(&peer, false).unwrap();

        // The writer's previous response only covered sequence 1; it reports applying 2 elsewhere.
        writer.state = DatatypeState::Subscribed;
        writer.transactions.clear();
        writer.checkpoint.sseq = 2;
        let pulled = server.process(&writer, false).unwrap();
        assert!(pulled.error.is_none());
        assert_eq!(server.synced.checkpoints[&writer.cuid].sseq, 2);
        assert!(pulled.transactions.is_empty());
    }

    #[test]
    fn can_keep_a_failed_snapshot_application_unacknowledged() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let creator = Client::builder(collection.clone(), "creator")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let reader = Client::builder(collection, "reader")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let source = creator
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();
        source.increase().unwrap();
        source.sync().unwrap();
        let replica = reader
            .subscribe_datatype(key)
            .with_readonly()
            .build_counter()
            .unwrap();
        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &reader.get_cuid())
            .unwrap();
        interceptor.set_after_pull(|pull| {
            // Force the actual snapshot application to fail after the backend supplied it.
            let snapshot = Arc::make_mut(pull.snapshot_transaction.as_mut().unwrap());
            snapshot.operations.clear();
            Ok(())
        });
        assert!(replica.sync().is_err());
        let server = connectivity
            .get_local_datatype_server(&resource_id)
            .unwrap();
        assert_eq!(server.read().synced.checkpoints[&reader.get_cuid()].sseq, 0);
        assert_eq!(server.read().synced.safe_sseq(), 0);
        assert_eq!(replica.get_server_version(), 0);
    }

    #[test]
    fn can_exclude_registration_only_handles_from_applied_progress() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let creator = Client::builder(collection.clone(), "creator")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let pending = Client::builder(collection, "pending")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let source = creator
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();
        source.increase().unwrap();
        source.sync().unwrap();
        source.sync().unwrap();
        let waiting = pending.subscribe_datatype(key).build_counter().unwrap();
        let server = connectivity
            .get_local_datatype_server(&resource_id)
            .unwrap();
        assert_eq!(server.read().synced.checkpoints.len(), 1);
        assert_eq!(server.read().synced.safe_sseq(), 1);
        let unsubscribe = PushPullPack::new(&waiting.get_attr(), DatatypeState::Unsubscribing);
        assert!(
            server
                .write()
                .process(&unsubscribe, false)
                .unwrap()
                .error
                .is_none()
        );
        assert!(!server.read().wired_map.contains_key(&pending.get_cuid()));
        source.unsubscribe().unwrap();
        source.sync().unwrap();
        assert!(
            connectivity
                .get_local_datatype_server(&resource_id)
                .is_none()
        );
    }

    #[test]
    fn can_restart_client_sequences_after_unsubscribe_and_resubscribe() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let client = Client::builder(collection.clone(), "rejoining")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let peer_client = Client::builder(collection, "remaining")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key.clone()).build_counter().unwrap();
        counter.increase_by(7).unwrap();
        counter.sync().unwrap();
        let peer = peer_client
            .subscribe_datatype(key.clone())
            .build_counter()
            .unwrap();
        peer.sync().unwrap();
        peer.increase_by(3).unwrap();
        peer.sync().unwrap();
        counter.sync().unwrap();

        counter.increase_by(2).unwrap();
        counter.unsubscribe().unwrap();
        counter.sync().unwrap();
        peer.sync().unwrap();
        assert_eq!(peer.get_value(), 12);
        let server = connectivity
            .get_local_datatype_server(&resource_id)
            .unwrap();
        {
            let server = server.read();
            assert!(!server.synced.checkpoints.contains_key(&client.get_cuid()));
            assert_eq!(
                server
                    .synced
                    .checkpoints
                    .get(&peer_client.get_cuid())
                    .map(|cp| cp.cseq),
                Some(1)
            );
        }

        let renewed = client.subscribe_datatype(key).build_counter().unwrap();
        renewed.sync().unwrap();
        assert_eq!(renewed.get_value(), 12);
        assert_eq!(renewed.get_client_version(), 0);
        renewed.increase_by(5).unwrap();
        renewed.sync().unwrap();
        assert_eq!(renewed.get_synced_client_version(), 1);
        peer.sync().unwrap();
        assert_eq!(peer.get_value(), 17);
        assert_eq!(renewed.get_value(), 17);
    }

    #[test]
    fn can_clear_client_progress_when_unsubscribe_rejects_pending_work() {
        let attr = new_attribute!(DataType::Counter);
        let mut server = LocalDatatypeServer::new(&attr);
        let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
        pushed.transactions = vec![transaction(
            &pushed.cuid,
            1,
            Operation::new_counter_increase(7),
        )];
        server.process(&pushed, false).unwrap();
        pushed.state = DatatypeState::Unsubscribing;
        pushed.is_readonly = true;
        pushed.transactions = vec![transaction(
            &pushed.cuid,
            2,
            Operation::new_counter_increase(5),
        )];
        let pulled = server.process(&pushed, false).unwrap();
        assert_eq!(pulled.error, Some(PushPullError::ReadonlyViolation));
        assert!(!server.synced.checkpoints.contains_key(&pushed.cuid));
        assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 7);
        assert_eq!(server.synced.sseq, 1);
        assert_eq!(server.synced.history.len(), 1);
        let repeated = server.process(&pushed, false).unwrap();
        assert!(repeated.error.is_none());
        assert_eq!(repeated.state, DatatypeState::Disabled);
        assert!(server.synced.checkpoints.is_empty());
        assert_eq!(server.synced.sseq, 1);
    }

    fn transaction(
        cuid: &crate::types::uid::Cuid,
        cseq: u64,
        mut operation: Operation,
    ) -> Arc<Transaction> {
        operation.set_lamport(cseq);
        let mut tx = Transaction::new(cuid, cseq);
        tx.push_operation(operation);
        Arc::new(tx)
    }

    #[rstest]
    #[case::duplicate(&[2, 3, 3, 4])]
    #[case::backward_sequence(&[2, 3, 2, 4])]
    fn can_reject_nonsequential_new_transactions_without_publishing_batch(
        #[case] sequences: &[u64],
    ) {
        let attr = new_attribute!(DataType::Counter);
        let mut server = LocalDatatypeServer::new(&attr);
        let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
        let accepted = transaction(&pushed.cuid, 1, Operation::new_counter_increase(7));
        pushed.transactions = vec![accepted.clone()];
        assert!(server.process(&pushed, false).unwrap().error.is_none());
        let before = server.synced.crdt.serialize();
        let history = server.synced.history.clone();
        let checkpoints = server.synced.checkpoints.clone();

        pushed.state = DatatypeState::Subscribed;
        pushed.checkpoint.sseq = 1;
        pushed.transactions = sequences
            .iter()
            .enumerate()
            .map(|(index, &cseq)| {
                transaction(
                    &pushed.cuid,
                    cseq,
                    Operation::new_counter_increase(index as i64 + 10),
                )
            })
            .collect();
        let failed = server.process(&pushed, false).unwrap();
        assert!(matches!(
            failed.error,
            Some(PushPullError::ProtocolViolation(_))
        ));
        assert_eq!(server.synced.crdt.serialize(), before);
        assert_eq!(server.synced.history, history);
        assert_eq!(server.synced.checkpoints, checkpoints);
        assert_eq!(server.synced.sseq, 1);
        assert_eq!(server.synced.lamport, 1);

        // A retry may still include transactions accepted by an earlier request.
        pushed.transactions = vec![
            accepted,
            transaction(&pushed.cuid, 2, Operation::new_counter_increase(10)),
            transaction(&pushed.cuid, 3, Operation::new_counter_increase(20)),
        ];
        assert!(server.process(&pushed, false).unwrap().error.is_none());
        assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 37);
        assert_eq!(server.synced.history.len(), 3);
        assert_eq!(server.synced.sseq, 3);
        assert_eq!(
            server.synced.checkpoints[&pushed.cuid],
            CheckPoint::new(1, 3)
        );
    }

    #[rstest]
    #[case(DataType::Counter)]
    #[case(DataType::Variable)]
    fn can_reject_a_late_batch_failure_without_advancing_backend_state(
        #[case] data_type: DataType,
    ) {
        let attr = new_attribute!(data_type);
        let mut server = LocalDatatypeServer::new(&attr);
        let operation = |value: i64| match data_type {
            DataType::Counter => Operation::new_counter_increase(value),
            DataType::Variable => {
                Operation::new_variable_set(value.to_string().into_bytes().into_boxed_slice())
            }
            _ => unreachable!(),
        };
        let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
        pushed.transactions = vec![transaction(&pushed.cuid, 1, operation(7))];
        assert!(server.process(&pushed, false).unwrap().error.is_none());
        let before = server.synced.crdt.serialize();
        let history = server.synced.history.clone();
        let checkpoints = server.synced.checkpoints.clone();
        pushed.state = DatatypeState::Subscribed;
        pushed.transactions = vec![
            transaction(&pushed.cuid, 2, operation(10)),
            transaction(&pushed.cuid, 3, operation(20)),
        ];
        let mut invalid = Operation::new_snapshot(Vec::new().into_boxed_slice());
        invalid.set_lamport(50);
        Arc::make_mut(&mut pushed.transactions[1]).push_operation(invalid);
        let failed = server.process(&pushed, false).unwrap();
        assert!(matches!(
            failed.error,
            Some(PushPullError::ProtocolViolation(_))
        ));
        assert_eq!(server.synced.crdt.serialize(), before);
        assert_eq!(server.synced.history, history);
        assert_eq!(server.synced.checkpoints, checkpoints);
        assert_eq!(server.synced.sseq, 1);
        assert_eq!(server.synced.lamport, 1);

        Arc::make_mut(&mut pushed.transactions[1]).operations.pop();
        assert!(server.process(&pushed, false).unwrap().error.is_none());
        assert_eq!(server.synced.sseq, 3);
        assert_eq!(server.synced.lamport, 3);
        let after = server.synced.crdt.serialize();
        assert!(server.process(&pushed, false).unwrap().error.is_none());
        pushed.transactions.clear();
        assert!(server.process(&pushed, false).unwrap().error.is_none());
        assert_eq!(server.synced.crdt.serialize(), after);
        assert_eq!(server.synced.history.len(), 3);
        assert_eq!(server.synced.sseq, 3);
    }

    #[rstest]
    #[case::wrong_operation(0, DatatypeState::Creating)]
    #[case::wrong_origin(1, DatatypeState::Creating)]
    #[case::sequence_gap(2, DatatypeState::Creating)]
    #[case::zero_lamport(3, DatatypeState::Creating)]
    #[case::nil_duid(4, DatatypeState::Creating)]
    #[case::nil_duid_subscribe_or_create(4, DatatypeState::SubscribingOrCreating)]
    fn can_keep_a_failed_creation_unpublished(#[case] mutation: u8, #[case] state: DatatypeState) {
        let attr = new_attribute!(DataType::Counter);
        let mut server = LocalDatatypeServer::new(&attr);
        let mut pushed = PushPullPack::new(&attr, state);
        let mut tx = (*transaction(&pushed.cuid, 1, Operation::new_counter_increase(7))).clone();
        match mutation {
            0 => {
                let mut invalid = Operation::new_variable_set(b"7".to_vec().into_boxed_slice());
                invalid.set_lamport(2);
                tx.push_operation(invalid);
            }
            1 => tx.cuid = crate::types::uid::Cuid::new(),
            2 => tx.cseq = 2,
            3 => tx.operations[0].set_lamport(0),
            4 => pushed.duid = Duid::new_nil(),
            _ => unreachable!(),
        }
        pushed.transactions.push(Arc::new(tx));
        let duid = server.duid.clone();
        assert!(server.process(&pushed, false).unwrap().error.is_some());
        assert!(!server.is_created());
        assert_eq!(server.duid, duid);
        assert!(server.synced.history.is_empty());
        assert!(server.synced.checkpoints.is_empty());
        assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 0);
        assert_eq!(server.synced.lamport, 0);
        assert_eq!(server.synced.sseq, 0);
        pushed.duid = attr.get_duid();
        pushed.transactions = vec![transaction(
            &pushed.cuid,
            1,
            Operation::new_counter_increase(7),
        )];
        assert!(server.process(&pushed, false).unwrap().error.is_none());
        assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 7);
        assert!(server.is_created());
    }

    #[test]
    fn can_serve_an_empty_backend_snapshot_before_any_writes() {
        let attr = new_attribute!(DataType::Counter);
        let mut server = LocalDatatypeServer::new(&attr);
        let create = PushPullPack::new(&attr, DatatypeState::Creating);
        assert!(server.process(&create, false).unwrap().error.is_none());
        let subscribe = PushPullPack::new(&attr, DatatypeState::Subscribing);
        let pulled = server.process(&subscribe, false).unwrap();
        let snapshot = pulled.snapshot_transaction.unwrap();
        assert_eq!(snapshot.sseq, 0);
        assert_eq!(snapshot.operations[0].lamport, 0);
        assert_eq!(snapshot.cuid, crate::types::uid::Cuid::new_nil());
        assert!(pulled.transactions.is_empty());
    }

    #[rstest]
    #[case::expired_handle(false)]
    #[case::busy_transaction(true)]
    fn can_serve_a_snapshot_without_accessing_client_state(#[case] keep_busy_handle: bool) {
        use crate::datatypes::{wired::WiredDatatype, wired_interceptor::WiredInterceptor};
        let attr = new_attribute!(DataType::Counter);
        let wired = WiredDatatype::new_arc_for_test(
            attr.clone(),
            DatatypeState::Subscribed,
            WiredInterceptor::new_arc(),
        );
        let mut server = LocalDatatypeServer::new(&attr);
        let (sender, _) = crossbeam_channel::unbounded();
        server.insert_client_item(wired.clone(), sender);
        let mut create = PushPullPack::new(&attr, DatatypeState::Creating);
        create.transactions = vec![transaction(
            &create.cuid,
            1,
            Operation::new_counter_increase(7),
        )];
        assert!(server.process(&create, false).unwrap().error.is_none());
        let handle = keep_busy_handle.then_some(wired);
        if let Some(wired) = &handle {
            wired
                .core
                .mutable
                .write()
                .execute_local_operation(Operation::new_counter_increase(5))
                .unwrap();
        } else {
            assert!(server.get_wired_datatype(&create.cuid).is_none());
        }
        let gate = handle.as_ref().map(|wired| wired.core.tx_mutex.lock());
        let mut subscribe = PushPullPack::new(&attr, DatatypeState::Subscribing);
        subscribe.cuid = crate::types::uid::Cuid::new();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            done_tx
                .send(server.process(&subscribe, false).unwrap())
                .unwrap();
        });
        let result = done_rx.recv_timeout(Duration::from_secs(5));
        drop(gate);
        worker.join().unwrap();
        let pulled = result.expect("subscription must not wait on the client's transaction gate");
        let snapshot = pulled.snapshot_transaction.unwrap();
        assert_eq!(snapshot.sseq, 1);
        assert_eq!(snapshot.operations[0].lamport, 1);
        let crate::operations::body::OperationBody::Snapshot(body) = &snapshot.operations[0].body
        else {
            panic!("expected snapshot");
        };
        let mut restored = crate::datatypes::Crdt::new(DataType::Counter);
        restored.deserialize(&body.data).unwrap();
        assert_eq!(restored.as_counter().unwrap().value(), 7);
        assert!(pulled.transactions.is_empty());
    }

    #[rstest]
    #[case::create(false)]
    #[case::subscribe_or_create(true)]
    fn can_subscribe_before_the_creator_applies_its_accepted_response(
        #[case] create_or_subscribe: bool,
    ) {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let creator = Client::builder(collection.clone(), "creator")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let subscriber = Client::builder(collection, "subscriber")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let builder = if create_or_subscribe {
            creator.subscribe_or_create_datatype(key.clone())
        } else {
            creator.create_datatype(key.clone())
        };
        let source = builder.build_counter().unwrap();
        source.increase_by(7).unwrap();
        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &creator.get_cuid())
            .unwrap();
        interceptor
            .set_after_pull(|_| Err(DatatypeError::SyncFailed("response lost".into()).mapping()));
        assert!(source.sync().is_err());
        assert_eq!(source.get_server_version(), 0);
        let replica = subscriber.subscribe_datatype(key).build_counter().unwrap();
        replica.sync().unwrap();
        assert_eq!(replica.get_value(), 7);
        assert_eq!(replica.get_server_version(), 1);
        replica.increase_by(3).unwrap();
        replica.sync().unwrap();
        interceptor.set_after_pull(|_| Ok(()));
        source.sync().unwrap();
        replica.sync().unwrap();
        assert_eq!(source.get_value(), 10);
        assert_eq!(replica.get_value(), 10);
        assert_eq!(source.get_server_version(), 2);
    }

    fn push_set_readonly(push: &mut PushPullPack) {
        push.is_readonly = true;
    }
    fn push_set_new_duid(push: &mut PushPullPack) {
        push.duid = Duid::new();
    }
    fn push_set_variable_type(push: &mut PushPullPack) {
        push.r#type = DataType::Variable;
    }
    fn push_set_disabled_state(push: &mut PushPullPack) {
        push.state = DatatypeState::Disabled;
    }
    fn push_add_transaction(push: &mut PushPullPack) {
        push.transactions.push(Arc::new(Transaction::default()));
    }

    fn assert_push_pull_pack(
        pulled: &PushPullPack,
        is_readonly: bool,
        cp: CheckPoint,
        error: Option<PushPullError>,
        tx_len: usize,
    ) {
        assert_eq!(pulled.is_readonly, is_readonly);
        assert_eq!(pulled.checkpoint, cp);
        if error.is_some() {
            assert_eq!(pulled.state, DatatypeState::Disabled);
        } else {
            assert_eq!(pulled.state, DatatypeState::Subscribed);
        }

        assert_eq!(pulled.error, error);
        assert_eq!(pulled.transactions.len(), tx_len);
    }

    #[rstest]
    #[case::readonly(
        false,
        push_set_readonly,
        true,
        Some(PushPullError::ReadonlyViolation),
        CheckPoint::new(0, 0),
        false,
        0
    )]
    #[case::normal(false, push_no_change, false, None, CheckPoint::new(10, 10), true, 10)]
    #[case::duplicate(true, push_no_change, false, None, CheckPoint::new(10, 10), true, 10)]
    #[case::already_created(
        true,
        push_set_new_duid,
        false,
        Some(PushPullError::CreateFailed("already exist".to_string())),
        CheckPoint::new(0, 0),
        true,
        10,
    )]
    #[instrument]
    fn can_process_creating(
        #[case] pre_create: bool,
        #[case] modify_push: fn(&mut PushPullPack),
        #[case] expected_is_readonly: bool,
        #[case] expected_error: Option<PushPullError>,
        #[case] expected_cp: CheckPoint,
        #[case] expected_created: bool,
        #[case] expected_history_len: usize,
    ) {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let resource_id = format!("{}/{}", get_test_collection_name!(), get_test_func_name!());
        let client1 = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        for i in 0..10 {
            counter1.increase_by(i).unwrap();
        }

        let server = connectivity
            .get_local_datatype_server(&resource_id)
            .unwrap();
        let wired_interceptor1 = connectivity
            .get_wired_interceptor(&resource_id, &client1.get_cuid())
            .unwrap();

        if pre_create {
            wired_interceptor1.set_after_pull(|_pull| {
                Err(ConnectivityError::TimedOut("".to_owned())
                    .to_datatype_error()
                    .mapping())
            });
            let _ = counter1.sync();
            assert!(server.read().is_created());
        }

        let expected_error = Arc::new(expected_error);
        let expected_error2 = expected_error.clone();

        wired_interceptor1
            .set_before_push(move |push| {
                modify_push(push);
                info!("PUSH:{push}");
            })
            .set_after_pull(move |pull| {
                info!("PULL:{pull}");
                assert_push_pull_pack(
                    pull,
                    expected_is_readonly,
                    expected_cp,
                    (*expected_error2).clone(),
                    0,
                );
                Ok(())
            });

        let sync_result = counter1.sync();
        if let Some(ref sppe) = *expected_error {
            let err = sync_result.unwrap_err();
            if matches!(sppe, PushPullError::ReadonlyViolation) {
                assert!(matches!(err, DatatypeError::ReadonlyViolation));
            } else {
                assert!(matches!(err, DatatypeError::ServerRejected(_)));
            }
            assert_eq!(counter1.get_state(), DatatypeState::Disabled);
        } else {
            assert!(sync_result.is_ok());
            assert_eq!(counter1.get_state(), DatatypeState::Subscribed);
        }
        assert_eq!(server.read().is_created(), expected_created);
        assert_eq!(server.read().synced.history.len(), expected_history_len);
        info!("{}", server.read());
    }

    #[rstest]
    #[case::success(true, push_no_change, None, CheckPoint::new(5, 0))]
    #[case::not_created(
        false,
        push_no_change,
        Some(PushPullError::ResourceNotFound("".to_string())),
        CheckPoint::new(0, 0),
    )]
    #[case::type_mismatch(
        true,
        push_set_variable_type,
        Some(PushPullError::ResourceNotFound("".to_string())),
        CheckPoint::new(0, 0),
    )]
    #[case::with_transactions(
        true,
        push_add_transaction,
        Some(PushPullError::ProtocolViolation("".to_string())),
        CheckPoint::new(0, 0),
    )]
    #[instrument]
    fn can_process_subscribing(
        #[case] creator_sync: bool,
        #[case] modify_push: fn(&mut PushPullPack),
        #[case] expected_error: Option<PushPullError>,
        #[case] expected_cp: CheckPoint,
    ) {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client1 = Client::builder(collection.clone(), key.clone())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(collection.clone(), key.clone())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();
        for i in 0..5 {
            counter1.increase_by(i).unwrap();
        }

        if creator_sync {
            assert!(counter1.sync().is_ok());
            assert_eq!(counter1.get_state(), DatatypeState::Subscribed);
        }

        let counter2 = client2
            .subscribe_datatype(key.clone())
            .build_counter()
            .unwrap();
        let interceptor2 = connectivity
            .get_wired_interceptor(&resource_id, &client2.get_cuid())
            .unwrap();

        let expected_error = Arc::new(expected_error);
        let expected_error2 = expected_error.clone();

        interceptor2
            .set_before_push(move |push| {
                modify_push(push);
                info!("PUSH: {push}");
            })
            .set_after_pull(move |pull| {
                info!("PULL: {pull}");
                assert_push_pull_pack(pull, false, expected_cp, (*expected_error2).clone(), 0);
                Ok(())
            });

        let sync_result = counter2.sync();
        if expected_error.is_some() {
            let err = sync_result.unwrap_err();
            assert!(matches!(err, DatatypeError::ServerRejected(_)));
            assert_eq!(counter2.get_state(), DatatypeState::Disabled);
        } else {
            assert!(sync_result.is_ok());
            assert_eq!(counter1.get_value(), counter2.get_value());
            assert_eq!(counter2.get_state(), DatatypeState::Subscribed);
            assert_ne!(
                counter1.get_attr().get_cuid(),
                counter2.get_attr().get_cuid()
            );
            assert_eq!(
                counter1.get_attr().get_duid(),
                counter2.get_attr().get_duid()
            );
        }
    }

    #[test]
    #[instrument]
    fn can_reject_readonly_unsubscribing_with_transactions() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client = Client::builder(collection, "client")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key).build_counter().unwrap();
        counter.sync().unwrap();
        counter.increase().unwrap();
        counter.unsubscribe().unwrap();

        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &client.get_cuid())
            .unwrap();
        interceptor
            .set_before_push(push_set_readonly)
            .set_after_pull(|pull| {
                assert_eq!(pull.state, DatatypeState::Disabled);
                assert_eq!(pull.error, Some(PushPullError::ReadonlyViolation));
                Ok(())
            });

        assert!(matches!(
            counter.sync().unwrap_err(),
            DatatypeError::ReadonlyViolation
        ));
        assert_eq!(counter.get_state(), DatatypeState::Disabled);
        assert!(
            connectivity
                .get_local_datatype_server(&resource_id)
                .is_none()
        );
        assert!(client.get_datatype(counter.get_key()).is_none());
    }

    #[rstest]
    #[case::normal(5, false, push_no_change, false, None, CheckPoint::new(10, 10), 0)]
    #[case::readonly_with_transactions(
        5,
        false,
        push_set_readonly,
        true,
        Some(PushPullError::ReadonlyViolation),
        CheckPoint::new(0, 0),
        0
    )]
    #[case::readonly_no_transactions(
        0,
        false,
        push_set_readonly,
        true,
        None,
        CheckPoint::new(5, 5),
        0
    )]
    #[case::pull_from_another_client(
        3,
        true,
        push_no_change,
        false,
        None,
        CheckPoint::new(8, 0),
        3
    )]
    #[instrument]
    fn can_process_subscribe(
        #[case] extra_ops: i64,
        #[case] use_subscriber: bool,
        #[case] modify_push: fn(&mut PushPullPack),
        #[case] expected_is_readonly: bool,
        #[case] expected_error: Option<PushPullError>,
        #[case] expected_cp: CheckPoint,
        #[case] expected_tx_len: usize,
    ) {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter1 = client1
            .subscribe_or_create_datatype(key.clone())
            .build_counter()
            .unwrap();
        for i in 0..5 {
            counter1.increase_by(i).unwrap();
        }
        assert!(counter1.sync().is_ok());
        assert_eq!(counter1.get_state(), DatatypeState::Subscribed);

        if use_subscriber {
            let client2 = Client::builder(collection, "client2")
                .with_connectivity(connectivity.clone())
                .build()
                .unwrap();
            let counter2 = client2.subscribe_datatype(key).build_counter().unwrap();
            assert!(counter2.sync().is_ok());
            assert_eq!(counter2.get_state(), DatatypeState::Subscribed);

            for i in 0..extra_ops {
                counter1.increase_by(i).unwrap();
            }
            assert!(counter1.sync().is_ok());

            let interceptor2 = connectivity
                .get_wired_interceptor(&resource_id, &client2.get_cuid())
                .unwrap();
            let expected_error = Arc::new(expected_error);
            let expected_error2 = expected_error.clone();
            interceptor2
                .set_before_push(move |push| {
                    modify_push(push);
                })
                .set_after_pull(move |pull| {
                    assert_push_pull_pack(
                        pull,
                        expected_is_readonly,
                        expected_cp,
                        (*expected_error2).clone(),
                        expected_tx_len,
                    );
                    Ok(())
                });
            check_error(expected_error, &counter2);
        } else {
            for i in 0..extra_ops {
                counter1.increase_by(i).unwrap();
            }
            let interceptor1 = connectivity
                .get_wired_interceptor(&resource_id, &client1.get_cuid())
                .unwrap();
            let expected_error = Arc::new(expected_error);
            let expected_error2 = expected_error.clone();
            interceptor1
                .set_before_push(move |push| {
                    info!("PUSH: {push}");
                    modify_push(push);
                })
                .set_after_pull(move |pull| {
                    info!("PULL: {pull}");
                    assert_push_pull_pack(
                        pull,
                        expected_is_readonly,
                        expected_cp,
                        (*expected_error2).clone(),
                        expected_tx_len,
                    );
                    Ok(())
                });
            check_error(expected_error, &counter1);
        }
    }

    fn check_error(expected_error: Arc<Option<PushPullError>>, counter: &Counter) {
        let sync_result = counter.sync();
        if expected_error.is_some() {
            assert!(matches!(
                sync_result.unwrap_err(),
                DatatypeError::ReadonlyViolation
            ));
            assert_eq!(counter.get_state(), DatatypeState::Disabled);
        } else {
            assert!(sync_result.is_ok());
            assert_eq!(counter.get_state(), DatatypeState::Subscribed);
        }
    }

    #[test]
    #[instrument]
    fn can_sync_bidirectionally_between_two_clients() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client1 = Client::builder(collection.clone(), "client1-1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(collection.clone(), "client1-2")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();

        let interceptor1 = connectivity
            .get_wired_interceptor(&resource_id, &client1.get_cuid())
            .unwrap();
        interceptor1
            .set_before_push(|push| {
                info!("counter1 PUSH:{push}");
            })
            .set_after_pull(|pull| {
                info!("counter1 PULL:{pull}");
                Ok(())
            });

        counter1.sync().unwrap();

        let counter2 = client2
            .subscribe_datatype(key.clone())
            .build_counter()
            .unwrap();

        let interceptor2 = connectivity
            .get_wired_interceptor(&resource_id, &client2.get_cuid())
            .unwrap();
        interceptor2
            .set_before_push(|push| {
                info!("counter2 PUSH:{push}");
            })
            .set_after_pull(|pull| {
                info!("counter2 PULL:{pull}");
                Ok(())
            });
        counter2.sync().unwrap();

        // === A: one-way convergence counter1 → counter2 ===
        for i in 0..5 {
            counter1.increase_by(i).unwrap();
        }
        // 0+1+2+3+4 = 10
        let expected_after_a = 10;
        assert_eq!(counter1.get_value(), expected_after_a);

        counter1.sync().unwrap();
        // counter1 must not re-apply its own transactions
        assert_eq!(counter1.get_value(), expected_after_a);

        counter2.sync().unwrap();
        assert_eq!(counter2.get_value(), expected_after_a);
        assert_eq!(counter1.get_value(), counter2.get_value());

        // === B: reverse convergence counter2 → counter1 ===
        counter2.increase_by(5).unwrap();
        counter2.increase_by(5).unwrap();
        let expected_after_b = expected_after_a + 10;

        counter2.sync().unwrap();
        assert_eq!(counter2.get_value(), expected_after_b);

        counter1.sync().unwrap();
        assert_eq!(counter1.get_value(), expected_after_b);

        // === C: bidirectional convergence after concurrent writes ===
        counter1.increase_by(3).unwrap();
        counter2.increase_by(7).unwrap();
        let expected_after_c = expected_after_b + 3 + 7;

        counter1.sync().unwrap();
        counter2.sync().unwrap();
        counter1.sync().unwrap(); // counter1 fetches counter2's tx
        assert_eq!(counter1.get_value(), expected_after_c);
        assert_eq!(counter2.get_value(), expected_after_c);

        // === D: checkpoint consistency and idempotent sync ===
        // both clients must see the same server state
        assert_eq!(counter1.get_server_version(), counter2.get_server_version());
        // all local transactions of each client must be confirmed by the server
        assert_eq!(
            counter1.get_client_version(),
            counter1.get_synced_client_version()
        );
        assert_eq!(
            counter2.get_client_version(),
            counter2.get_synced_client_version()
        );

        // repeated syncs must not change values (idempotent)
        let value_before = counter1.get_value();
        counter1.sync().unwrap();
        counter2.sync().unwrap();
        assert_eq!(counter1.get_value(), value_before);
        assert_eq!(counter2.get_value(), value_before);
    }

    #[test]
    #[instrument]
    fn can_reject_subscribed_push_from_unsubscribed_client() {
        use std::time::Duration;

        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client1 = Client::builder(collection.clone(), get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();

        counter1.sync().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::Subscribed);

        // Simulate the server losing the client's subscription (e.g., after a server restart).
        connectivity.remove_client_subscription(&resource_id, &client1.get_cuid());

        // The next sync must fail with ServerRejected and disable the datatype.
        let result = counter1.sync();
        assert!(matches!(
            result.unwrap_err(),
            DatatypeError::ServerRejected(_)
        ));
        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| counter1.get_state() == DatatypeState::Disabled);
    }

    #[test]
    #[instrument]
    fn can_unsubscribe_not_subscribed_client_gracefully() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client1 = Client::builder(collection.clone(), get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();

        counter1.sync().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::Subscribed);

        // Simulate the server losing the client's subscription.
        connectivity.remove_client_subscription(&resource_id, &client1.get_cuid());

        // Unsubscribing a client whose server-side subscription is gone must complete
        // gracefully: no error, and the datatype reaches Disabled.
        // Manual mode: send_push_transaction_with_best_effort skips the event, so call
        // sync() explicitly to drive the Unsubscribing → Disabled transition.
        counter1.unsubscribe().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::Unsubscribing);
        counter1.sync().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::Disabled);
    }

    #[test]
    #[instrument]
    fn can_reject_type_mismatch_in_subscribed_push() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client = Client::builder(collection.clone(), get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key.clone()).build_counter().unwrap();
        counter.sync().unwrap();
        assert_eq!(counter.get_state(), DatatypeState::Subscribed);

        // Simulate a client sending a push with a different DataType after subscription.
        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &client.get_cuid())
            .unwrap();
        interceptor.set_before_push(push_set_variable_type);

        let result = counter.sync();
        assert!(matches!(
            result.unwrap_err(),
            DatatypeError::ServerRejected(_)
        ));
        assert_eq!(counter.get_state(), DatatypeState::Disabled);
    }

    #[test]
    #[instrument]
    fn can_reject_push_from_disabled_client() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client = Client::builder(collection.clone(), get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter = client.create_datatype(key.clone()).build_counter().unwrap();
        counter.sync().unwrap();
        assert_eq!(counter.get_state(), DatatypeState::Subscribed);

        // Simulate a buggy or malicious client sending a push with Disabled state.
        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &client.get_cuid())
            .unwrap();
        interceptor.set_before_push(push_set_disabled_state);

        let result = counter.sync();
        assert!(matches!(
            result.unwrap_err(),
            DatatypeError::ServerRejected(_)
        ));
        assert_eq!(counter.get_state(), DatatypeState::Disabled);
    }

    #[test]
    #[instrument]
    fn can_subscribe_from_backend_state_after_clients_are_removed() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        // The backend retains accepted state independently of client registrations.
        let client1 = Client::builder(collection.clone(), "creator")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();
        counter1.increase_by(7).unwrap();
        counter1.sync().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::Subscribed);

        // A second subscriber receives the accepted state.
        let client2 = Client::builder(collection.clone(), "subscriber")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter2 = client2
            .subscribe_datatype(key.clone())
            .build_counter()
            .unwrap();
        counter2.sync().unwrap();
        assert_eq!(counter2.get_state(), DatatypeState::Subscribed);

        connectivity.remove_client_subscription(&resource_id, &client1.get_cuid());
        connectivity.remove_client_subscription(&resource_id, &client2.get_cuid());

        let client3 = Client::builder(collection.clone(), "new-subscriber")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter3 = client3
            .subscribe_datatype(key.clone())
            .build_counter()
            .unwrap();
        counter3.sync().unwrap();
        assert_eq!(counter3.get_state(), DatatypeState::Subscribed);
        assert_eq!(counter3.get_value(), 7);
    }
}
