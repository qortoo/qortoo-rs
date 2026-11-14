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
    DatatypeState,
    connectivity::{Connectivity, local_datatype_server::LocalDatatypeServer},
    datatypes::{event_loop::Event, wired::WiredDatatype},
    errors::{connectivity::ConnectivityError, messages::sync as msg, push_pull::PushPullError},
    types::{common::ResourceID, push_pull_pack::PushPullPack, uid::Cuid},
};

#[derive(Default)]
struct LocalDatatypeServers {
    active: HashMap<ResourceID, Arc<RwLock<LocalDatatypeServer>>>,
    retired_sub_gens: HashMap<ResourceID, HashMap<Cuid, u64>>,
}

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
/// Client records retain the maximum subscription generation per resource for this
/// backend's lifetime, including after unsubscribe or resource removal. This costs
/// one upper bound per resource/CUID; a new backend clears these in-memory records.
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
    datatype_servers: RwLock<LocalDatatypeServers>,
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
            datatype_servers: RwLock::default(),
            is_realtime: AtomicBool::new(true),
        })
    }

    /// Returns the local datatype server for a given resource ID, if it exists.
    pub(crate) fn get_local_datatype_server(
        &self,
        resource_id: &str,
    ) -> Option<Arc<RwLock<LocalDatatypeServer>>> {
        let datatypes = self.datatype_servers.read();
        datatypes.active.get(resource_id).cloned()
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

    #[cfg(test)]
    pub fn remove_client_subscription(
        &self,
        resource_id: &ResourceID,
        cuid: &crate::types::uid::Cuid,
    ) {
        if let Some(server) = self.get_local_datatype_server(resource_id) {
            server.write().remove_client_subscription(cuid);
        }
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
        let attr = wired.core.attr.clone();
        let resource_id = attr.resource_id();

        let mut datatypes = self.datatype_servers.write();
        let LocalDatatypeServers {
            active,
            retired_sub_gens,
        } = &mut *datatypes;
        let server = active.entry(resource_id.clone()).or_insert_with(|| {
            let max_sub_gens = retired_sub_gens.remove(&resource_id).unwrap_or_default();
            Arc::new(RwLock::new(LocalDatatypeServer::new(&attr, max_sub_gens)))
        });
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

        // Keep registration and removal from replacing the resource during processing.
        let datatypes = self.datatype_servers.read();
        let Some(server_with_lock) = datatypes.active.get(&resource_id).cloned() else {
            let mut pulled = pushed.get_pulled_stub();
            let retired = datatypes
                .retired_sub_gens
                .get(&resource_id)
                .and_then(|generations| generations.get(&pushed.cuid))
                .is_some_and(|generation| pushed.sub_gen <= *generation);
            pulled.error = match (retired, pushed.state) {
                (true, DatatypeState::Unsubscribing) => None,
                (true, _) => Some(PushPullError::MissingSubscription(
                    msg::RETIRED_SUBSCRIPTION.into(),
                )),
                (false, _) => Some(PushPullError::ResourceNotFound(resource_id.clone())),
            };
            pulled.state = DatatypeState::Disabled;
            return Ok(pulled);
        };
        let (pulled, should_remove_server) = {
            let mut server = server_with_lock.write();
            let pulled = server.process(pushed, self.is_realtime())?;
            (
                pulled,
                pushed.state == DatatypeState::Unsubscribing && server.is_empty(),
            )
        };

        drop(datatypes);
        if should_remove_server {
            let mut datatypes = self.datatype_servers.write();
            if datatypes
                .active
                .get(&resource_id)
                .is_some_and(|server| Arc::ptr_eq(server, &server_with_lock))
            {
                let mut server = server_with_lock.write();
                if server.is_empty() {
                    let max_sub_gens = std::mem::take(&mut server.max_sub_gens);
                    datatypes
                        .retired_sub_gens
                        .insert(resource_id.clone(), max_sub_gens);
                    datatypes.active.remove(&resource_id);
                }
            }
        }

        Ok(pulled)
    }

    fn is_realtime(&self) -> bool {
        self.is_realtime.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests;
