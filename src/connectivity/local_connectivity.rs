use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    fmt::{Debug, Formatter},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crossbeam_channel::Sender;
use parking_lot::RwLock;

use crate::{
    ClientError, CollectionInfo, DatatypeState, SnapshotWriteOptions,
    connectivity::{Connectivity, local_datatype_server::LocalDatatypeServer},
    datatypes::{event_loop::Event, wired::WiredDatatype},
    errors::{connectivity::ConnectivityError, messages::sync as msg, push_pull::PushPullError},
    types::{common::ResourceID, push_pull_pack::PushPullPack, uid::Cuid},
    utils::name_validator::is_valid_collection_name,
};

struct LocalCollection {
    info: Arc<CollectionInfo>,
    clients: HashSet<Cuid>,
}

impl LocalCollection {
    fn new(info: CollectionInfo) -> Self {
        Self {
            info: Arc::new(info),
            clients: HashSet::new(),
        }
    }
}

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
/// The backend retains opened client identities per collection and maximum
/// subscription generations per resource for its lifetime.
///
/// # Examples
///
/// ```
/// # #[cfg(all(feature = "snapshot-bitcode", feature = "snapshot-zstd"))]
/// # {
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
///     .build_and_open()
///     .unwrap();
///
/// let client2 = Client::builder("my-collection", "client-2")
///     .with_connectivity(connectivity)
///     .build_and_open()
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
/// # }
/// ```
#[allow(dead_code)]
pub struct LocalConnectivity {
    collections: RwLock<HashMap<String, LocalCollection>>,
    datatype_servers: RwLock<LocalDatatypeServers>,
    is_realtime: AtomicBool,
    default_snapshot_write_options: SnapshotWriteOptions,
}

impl LocalConnectivity {
    /// Creates a new `LocalConnectivity` instance wrapped in an `Arc`.
    ///
    /// The returned instance starts in realtime mode, meaning changes
    /// are automatically synchronized across clients.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(all(feature = "snapshot-bitcode", feature = "snapshot-zstd"))]
    /// # {
    /// use qortoo::LocalConnectivity;
    ///
    /// let connectivity = LocalConnectivity::new_arc();
    /// # }
    /// ```
    #[cfg(all(feature = "snapshot-bitcode", feature = "snapshot-zstd"))]
    pub fn new_arc() -> Arc<Self> {
        Self::new_arc_with_snapshot_options(Default::default())
    }

    /// Creates a backend with explicit default snapshot writer options for its collections.
    /// Only capabilities enabled by Cargo features can be selected.
    pub fn new_arc_with_snapshot_options(options: SnapshotWriteOptions) -> Arc<Self> {
        Arc::new(Self {
            collections: RwLock::default(),
            datatype_servers: RwLock::default(),
            is_realtime: AtomicBool::new(true),
            default_snapshot_write_options: options,
        })
    }

    /// Creates a collection with immutable information, including snapshot writer options.
    ///
    /// Call this before opening the first client for the collection. If the writer
    /// options are omitted, the collection uses the backend's defaults.
    ///
    /// # Errors
    ///
    /// Returns an error if the name or limits are invalid, or if the collection already exists.
    pub fn create_collection(
        &self,
        name: &str,
        mut info: CollectionInfo,
    ) -> Result<(), ClientError> {
        if !is_valid_collection_name(name) {
            return Err(ClientError::invalid_collection_name(name));
        }
        info.validate_limits()?;
        info.snapshot_write_options
            .get_or_insert(self.default_snapshot_write_options);

        match self.collections.write().entry(name.to_owned()) {
            Entry::Vacant(entry) => {
                entry.insert(LocalCollection::new(info));
                Ok(())
            }
            Entry::Occupied(_) => Err(ClientError::CollectionAlreadyExists(name.to_owned())),
        }
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
    /// # #[cfg(all(feature = "snapshot-bitcode", feature = "snapshot-zstd"))]
    /// # {
    /// use qortoo::LocalConnectivity;
    ///
    /// let connectivity = LocalConnectivity::new_arc();
    /// connectivity.set_realtime(false); // Switch to manual sync mode
    /// # }
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
    fn open_client(
        &self,
        collection: &str,
        cuid: &Cuid,
    ) -> Result<Arc<CollectionInfo>, ConnectivityError> {
        let mut collections = self.collections.write();
        let collection = collections.entry(collection.to_owned()).or_insert_with(|| {
            LocalCollection::new(CollectionInfo {
                snapshot_write_options: Some(self.default_snapshot_write_options),
                ..Default::default()
            })
        });
        collection.clients.insert(cuid.clone());
        Ok(collection.info.clone())
    }

    fn register(&self, wired: Arc<WiredDatatype>, sender: Sender<Event>) {
        let attr = wired.core.attr.clone();
        let resource_id = attr.resource_id();
        let options = self
            .collections
            .read()
            .get(attr.client_common.collection.as_ref())
            .expect("client must open its collection before registering a datatype")
            .info
            .snapshot_write_options
            .expect("local collection must have snapshot writer options");

        let mut datatypes = self.datatype_servers.write();
        let LocalDatatypeServers {
            active,
            retired_sub_gens,
        } = &mut *datatypes;
        let server = active.entry(resource_id.clone()).or_insert_with(|| {
            let max_sub_gens = retired_sub_gens.remove(&resource_id).unwrap_or_default();
            Arc::new(RwLock::new(LocalDatatypeServer::new(
                &attr,
                max_sub_gens,
                options,
            )))
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
