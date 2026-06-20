use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crossbeam_channel::Sender;
use parking_lot::RwLock;

use crate::{
    ConnectivityError, DatatypeState,
    connectivity::{Connectivity, local_datatype_server::LocalDatatypeServer},
    datatypes::{event_loop::Event, wired::WiredDatatype},
    types::{common::ResourceID, push_pull_pack::PushPullPack},
};

/// An in-memory connectivity backend for local testing and development.
///
/// `LocalConnectivity` simulates a synchronization server entirely in-process,
/// allowing multiple clients to share and synchronize datatypes without any
/// network communication. This is useful for:
///
/// - **Unit testing**: Test CRDT synchronization behavior without external dependencies
/// - **Development**: Prototype applications before connecting to a real backend
/// - **Demonstrations**: Show synchronization concepts in a controlled environment
///
/// # Realtime Mode
///
/// By default, `LocalConnectivity` operates in realtime mode, where changes
/// are automatically synchronized. Use [`set_realtime(false)`](Self::set_realtime)
/// to switch to manual mode, requiring explicit [`sync()`](crate::Datatype::sync) calls.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use qortoo::{Client, Datatype, DatatypeState, LocalConnectivity};
///
/// // Create a shared local connectivity backend
/// let connectivity = LocalConnectivity::new_arc();
/// connectivity.set_realtime(false); // Manual sync mode
///
/// // Create two clients sharing the same backend
/// let client1 = Client::builder("my-collection", "client-1")
///     .with_connectivity(connectivity.clone())
///     .build()
///     .unwrap();
///
/// let client2 = Client::builder("my-collection", "client-2")
///     .with_connectivity(connectivity)
///     .build()
///     .unwrap();
///
/// // Create a counter in client1
/// let counter1 = client1.create_datatype("shared-counter").build_counter().unwrap();
/// counter1.increase().unwrap();
/// counter1.sync().unwrap();
///
/// // Subscribe to the same counter in client2
/// let counter2 = client2.subscribe_datatype("shared-counter").build_counter().unwrap();
/// counter2.sync().unwrap();
///
/// // Both clients see the same value
/// assert_eq!(counter2.get_value(), 1);
/// ```
#[allow(dead_code)]
pub struct LocalConnectivity {
    datatype_servers: RwLock<HashMap<ResourceID, Arc<RwLock<LocalDatatypeServer>>>>,
    is_realtime: AtomicBool,
}

impl LocalConnectivity {
    /// Creates a new `LocalConnectivity` instance wrapped in an `Arc`.
    ///
    /// The returned instance starts in realtime mode, meaning changes
    /// are automatically synchronized across connected clients.
    ///
    /// # Examples
    ///
    /// ```
    /// use qortoo::LocalConnectivity;
    ///
    /// let connectivity = LocalConnectivity::new_arc();
    /// ```
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self {
            datatype_servers: RwLock::new(HashMap::new()),
            is_realtime: AtomicBool::new(true),
        })
    }

    /// Returns the local datatype server for a given resource ID, if it exists.
    pub(crate) fn get_local_datatype_server(
        &self,
        resource_id: &str,
    ) -> Option<Arc<RwLock<LocalDatatypeServer>>> {
        let datatypes = self.datatype_servers.read();
        datatypes.get(resource_id).cloned()
    }

    /// Sets whether this connectivity operates in realtime mode.
    ///
    /// - **Realtime mode (`true`)**: Changes are automatically synchronized
    ///   via the event loop. This is the default behavior.
    /// - **Manual mode (`false`)**: Changes require explicit [`sync()`](crate::Datatype::sync)
    ///   calls to synchronize. Useful for testing synchronization behavior.
    ///
    /// # Arguments
    ///
    /// * `tf` - `true` for realtime mode, `false` for manual mode
    ///
    /// # Examples
    ///
    /// ```
    /// use qortoo::LocalConnectivity;
    ///
    /// let connectivity = LocalConnectivity::new_arc();
    /// connectivity.set_realtime(false); // Switch to manual sync mode
    /// ```
    pub fn set_realtime(&self, tf: bool) {
        self.is_realtime.store(tf, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub fn get_wired_interceptor(
        &self,
        resource_id: &ResourceID,
        cuid: &crate::types::uid::Cuid,
    ) -> Option<Arc<crate::datatypes::wired_interceptor::WiredInterceptor>> {
        let server = self.get_local_datatype_server(resource_id)?;
        let wired_datatype = server.read().get_wired_datatype(cuid)?;
        Some(wired_datatype.get_wired_interceptor())
    }
}

impl Debug for LocalConnectivity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalConnectivity")
            // .field("datatype_servers", &self.datatype_servers)
            .finish()
    }
}

impl Connectivity for LocalConnectivity {
    fn register(&self, wired: Arc<WiredDatatype>, sender: Sender<Event>) {
        let attr = wired.attr.clone();
        let resource_id = attr.resource_id();

        let server = {
            let mut datatypes = self.datatype_servers.write();
            datatypes
                .entry(resource_id)
                .or_insert_with(|| Arc::new(RwLock::new(LocalDatatypeServer::new(&attr))))
                .clone()
        };

        server.write().insert_client_item(wired, sender);
    }

    #[tracing::instrument(name = "LocalConnectivity::push_pull", skip_all, fields(
        collection=%pushed.collection,
        cuid=%pushed.cuid,
        duid=%pushed.duid,
        key=%pushed.key,
    ))]
    fn push_pull(&self, pushed: &PushPullPack) -> Result<PushPullPack, ConnectivityError> {
        let resource_id = pushed.resource_id();

        let server_with_lock = self
            .get_local_datatype_server(&resource_id)
            .ok_or_else(|| ConnectivityError::ResourceNotFound(resource_id.clone()))?;
        let (pulled, should_remove_server) = {
            let mut server = server_with_lock.write();
            let pulled = match pushed.state {
                DatatypeState::DueToCreate => server.process_due_to_create(pushed)?,
                DatatypeState::DueToSubscribe => server.process_due_to_subscribe(pushed)?,
                DatatypeState::DueToSubscribeOrCreate => {
                    server.process_due_to_subscribe_or_create(pushed)?
                }
                DatatypeState::Subscribed => {
                    server.process_subscribed(pushed, self.is_realtime())?
                }
                DatatypeState::DueToUnsubscribe => {
                    server.process_due_to_unsubscribe(pushed, self.is_realtime())?
                }
                DatatypeState::DueToDelete => server.process_due_to_delete(pushed)?,
                DatatypeState::Disabled => server.process_disabled(pushed)?,
            };
            (
                pulled,
                pushed.state == DatatypeState::DueToUnsubscribe && server.is_empty(),
            )
        };

        if should_remove_server {
            let mut datatypes = self.datatype_servers.write();
            if datatypes
                .get(&resource_id)
                .is_some_and(|server| Arc::ptr_eq(server, &server_with_lock))
            {
                datatypes.remove(&resource_id);
            }
        }

        Ok(pulled)
    }

    fn is_realtime(&self) -> bool {
        self.is_realtime.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests_local_connectivity {
    use std::time::Duration;

    use tracing::instrument;

    use crate::{
        Client, Datatype, DatatypeState,
        connectivity::local_connectivity::LocalConnectivity,
        utils::test_utils::{get_test_collection_name, get_test_func_name, get_test_ids},
    };

    #[test]
    #[instrument]
    fn can_compare_manual_and_realtime_local_connectivity() {
        let lc_manual = LocalConnectivity::new_arc();
        lc_manual.set_realtime(false);
        let client_manual = Client::builder(get_test_collection_name!(), "manual client")
            .with_connectivity(lc_manual)
            .build()
            .unwrap();
        let counter_manual = client_manual
            .create_datatype("manual")
            .build_counter()
            .unwrap();

        let lc_realtime = LocalConnectivity::new_arc();
        let client_realtime = Client::builder(get_test_collection_name!(), "realtime client")
            .with_connectivity(lc_realtime)
            .build()
            .unwrap();
        let counter_realtime = client_realtime
            .create_datatype("realtime")
            .build_counter()
            .unwrap();

        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| counter_realtime.get_state() == DatatypeState::Subscribed);
        assert_ne!(counter_realtime.get_state(), counter_manual.get_state());

        counter_manual.sync().unwrap();
        assert_eq!(counter_manual.get_state(), DatatypeState::Subscribed);
    }

    #[test]
    #[instrument]
    fn can_notify_other_clients_after_realtime_push() {
        let connectivity = LocalConnectivity::new_arc();
        let (collection, key, _) = get_test_ids!();
        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(collection, "client2")
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();
        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| counter1.get_state() == DatatypeState::Subscribed);

        let counter2 = client2.subscribe_datatype(key).build_counter().unwrap();
        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| counter2.get_state() == DatatypeState::Subscribed);

        counter1.increase_by(7).unwrap();

        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| counter2.get_value() == 7);
        assert_eq!(counter1.get_server_version(), counter2.get_server_version());
    }

    #[test]
    #[instrument]
    fn does_not_notify_other_clients_in_manual_mode() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, _) = get_test_ids!();
        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(collection, "client2")
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();
        counter1.sync().unwrap();

        let counter2 = client2.subscribe_datatype(key).build_counter().unwrap();
        counter2.sync().unwrap();

        counter1.increase_by(7).unwrap();
        counter1.sync().unwrap();

        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(counter2.get_value(), 0);

        counter2.sync().unwrap();
        assert_eq!(counter2.get_value(), 7);
    }

    #[test]
    #[instrument]
    fn can_promote_remaining_client_when_creator_unsubscribes() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(collection.clone(), "client2")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client3 = Client::builder(collection, "client3")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();
        counter1.increase_by(7).unwrap();
        counter1.sync().unwrap();

        let counter2 = client2
            .subscribe_datatype(key.clone())
            .build_counter()
            .unwrap();
        counter2.sync().unwrap();
        assert_eq!(counter2.get_value(), 7);

        counter1.unsubscribe().unwrap();
        counter1.sync().unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::Disabled);

        let server = connectivity
            .get_local_datatype_server(&resource_id)
            .unwrap();
        assert_eq!(server.read().creator(), &client2.get_cuid());

        counter2.increase_by(5).unwrap();
        counter2.sync().unwrap();

        let counter3 = client3.subscribe_datatype(key).build_counter().unwrap();
        counter3.sync().unwrap();
        assert_eq!(counter3.get_value(), 12);
    }

    #[test]
    #[instrument]
    fn can_remove_server_when_last_client_unsubscribes() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(collection, "client2")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(key.clone())
            .build_counter()
            .unwrap();
        counter1.increase_by(7).unwrap();
        counter1.sync().unwrap();

        counter1.unsubscribe().unwrap();
        counter1.sync().unwrap();
        assert!(
            connectivity
                .get_local_datatype_server(&resource_id)
                .is_none()
        );

        let counter2 = client2.create_datatype(key).build_counter().unwrap();
        counter2.sync().unwrap();
        assert_eq!(counter2.get_value(), 0);
        assert_eq!(counter2.get_state(), DatatypeState::Subscribed);
    }
}
