use std::{
    collections::HashMap,
    num::NonZeroUsize,
    sync::{Arc, OnceLock},
    thread::available_parallelism,
};

use parking_lot::Mutex;
use tokio::runtime::{Builder, Handle, Runtime};

use crate::{defaults, observability::macros::add_span_event};

type RuntimeMap = HashMap<String, Runtime>;
type SharedRuntimeMap = Arc<Mutex<RuntimeMap>>;

static RUNTIME_MAP: OnceLock<SharedRuntimeMap> = OnceLock::new();

pub fn get_or_init_runtime_handle(group: &str) -> Handle {
    const THREAD_PREFIX: &str = "syncyam-";
    let map = RUNTIME_MAP.get_or_init(|| Arc::new(Mutex::new(HashMap::new())));
    let mut map_guard = map.lock();
    match map_guard.get(group) {
        Some(rt) => rt.handle().clone(),
        None => {
            let num_of_workers: usize = available_parallelism()
                .unwrap_or(NonZeroUsize::new(defaults::DEFAULT_THREAD_WORKERS).unwrap())
                .into();
            let rt = Builder::new_multi_thread()
                .enable_all()
                .worker_threads(num_of_workers)
                .thread_name(format!("{THREAD_PREFIX}{group}"))
                .build()
                .unwrap();
            let handle = rt.handle().clone();
            map_guard.insert(group.to_string(), rt);
            handle
        }
    }
}

#[allow(dead_code)]
pub fn reserve_to_shutdown_runtime(group: &str) {
    if let Some(map) = RUNTIME_MAP.get() {
        let mut map_guard = map.lock();
        let rt = map_guard.remove(group);
        if let Some(rt) = rt {
            let tasks = rt.metrics().num_alive_tasks();
            rt.shutdown_background();
            add_span_event!("shutdown runtime", "group"=>group, "tasks"=> tasks);
        }
    }
}

#[cfg(test)]
mod tests_runtime {
    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    use parking_lot::Mutex;
    use tokio::time::sleep;
    use tracing::info;

    use crate::utils::runtime::{get_or_init_runtime_handle, reserve_to_shutdown_runtime};

    #[test]
    fn can_show_how_to_work_runtime_in_sync_function() {
        let h1 = get_or_init_runtime_handle("sync_group");
        let h2 = get_or_init_runtime_handle("sync_group");
        h1.spawn(async {
            info!("h1 thread in {:?}", thread::current());
        });
        h2.spawn(async {
            info!("h2 thread in {:?}", thread::current());
        });
        h2.spawn(async {
            sleep(Duration::from_secs(1)).await;
        });
        reserve_to_shutdown_runtime("sync_group");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn can_show_how_to_work_runtime_in_async_function() {
        let h1 = get_or_init_runtime_handle("async_group");
        let h2 = get_or_init_runtime_handle("async_group");
        h1.spawn(async {
            info!("h1 thread in {:?}", thread::current());
        });
        h2.spawn(async {
            info!("h2 thread in {:?}", thread::current());
        });
        h2.spawn(async {
            sleep(Duration::from_secs(1)).await;
        });
        reserve_to_shutdown_runtime("async_group");
    }

    #[test]
    fn can_execute_runtimes_concurrently() {
        let start = Instant::now();
        let sleep_duration = Duration::from_millis(100);
        let cnt = Arc::new(Mutex::new(0));

        {
            let handle = get_or_init_runtime_handle("test_runtime1");
            let cnt = cnt.clone();
            handle.spawn(async move {
                for _i in 0..10 {
                    tokio::time::sleep(sleep_duration).await;
                    let mut cnt = cnt.lock();
                    *cnt += 1;
                }
            });
        }

        {
            let handle = get_or_init_runtime_handle("test_runtime2");
            let cnt = cnt.clone();
            handle.spawn(async move {
                for _i in 0..20 {
                    tokio::time::sleep(sleep_duration).await;
                    let mut cnt = cnt.lock();
                    *cnt += 1;
                }
            });
        }

        let cnt = cnt.clone();
        awaitility::at_most(Duration::from_secs(5))
            .poll_interval(Duration::from_millis(100))
            .until(move || {
                let cnt = cnt.lock();
                *cnt >= 20 // meet this condition only in one sec;
            });
        reserve_to_shutdown_runtime("test_runtime1");
        reserve_to_shutdown_runtime("test_runtime2");

        assert!(start.elapsed().as_secs() < 2);
    }
}
