use crate::datatypes::common::Attribute;

// --- Metric name constants (Prometheus naming convention) ---

const SYNC_TOTAL: &str = "qortoo_sync_total";
const SYNC_DURATION_SECONDS: &str = "qortoo_sync_duration_seconds";
const BACKOFF_TOTAL: &str = "qortoo_backoff_total";

// --- Label key constants ---

const LABEL_COLLECTION: &str = "collection";
const LABEL_KEY: &str = "key";
const LABEL_TYPE: &str = "type";
const LABEL_RESULT: &str = "result";

// --- Label value constants ---

const RESULT_SUCCESS: &str = "success";
const RESULT_FAILURE: &str = "failure";

// --- Helper functions ---

pub fn emit_sync(attr: &Attribute, success: bool, duration: std::time::Duration) {
    let result = if success {
        RESULT_SUCCESS
    } else {
        RESULT_FAILURE
    };
    let col = attr.client_common.collection.to_string();
    let key = attr.key.to_string();
    let dt = attr.r#type.to_string();
    metrics::counter!(
        SYNC_TOTAL,
        LABEL_COLLECTION => col.clone(),
        LABEL_KEY => key.clone(),
        LABEL_TYPE => dt.clone(),
        LABEL_RESULT => result,
    )
    .increment(1);
    metrics::histogram!(
        SYNC_DURATION_SECONDS,
        LABEL_COLLECTION => col,
        LABEL_KEY => key,
        LABEL_TYPE => dt,
    )
    .record(duration.as_secs_f64());
}

pub fn emit_backoff(attr: &Attribute) {
    metrics::counter!(
        BACKOFF_TOTAL,
        LABEL_COLLECTION => attr.client_common.collection.to_string(),
        LABEL_KEY => attr.key.to_string(),
        LABEL_TYPE => attr.r#type.to_string(),
    )
    .increment(1);
}

#[cfg(test)]
mod tests_metrics {
    use std::sync::OnceLock;

    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
    use tracing::instrument;

    use crate::{
        Client, ConnectivityError,
        connectivity::local_connectivity::LocalConnectivity,
        datatypes::datatype::Datatype,
        errors::push_pull::ClientPushPullError,
        utils::test_utils::{get_test_collection_name, get_test_func_name, get_test_ids},
    };

    static SNAPSHOTTER: OnceLock<Snapshotter> = OnceLock::new();

    fn snapshotter() -> &'static Snapshotter {
        SNAPSHOTTER.get_or_init(|| {
            let recorder = DebuggingRecorder::new();
            let snapshotter = recorder.snapshotter();
            let _ = recorder.install();
            snapshotter
        })
    }

    // snapshot() drains ALL registered metrics globally, not just the queried one.
    macro_rules! drain {
        ($s:expr) => {
            $s.snapshot().into_vec()
        };
    }

    macro_rules! extract_counter {
        ($vec:expr, $name:expr, $lk:expr, $lv:expr) => {{
            $vec.iter()
                .filter(|(ck, _, _, _)| {
                    ck.key().name() == $name
                        && ck
                            .key()
                            .labels()
                            .any(|l| l.key() == $lk && l.value() == $lv)
                })
                .map(|(_, _, _, v)| match v {
                    DebugValue::Counter(n) => *n,
                    _ => 0,
                })
                .sum::<u64>()
        }};
        ($vec:expr, $name:expr) => {{
            $vec.iter()
                .filter(|(ck, _, _, _)| ck.key().name() == $name)
                .map(|(_, _, _, v)| match v {
                    DebugValue::Counter(n) => *n,
                    _ => 0,
                })
                .sum::<u64>()
        }};
    }

    #[test]
    #[serial_test::serial]
    #[instrument]
    fn can_record_sync_success() {
        let s = snapshotter();
        drain!(s);

        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build()
            .unwrap();
        let counter = client.create_datatype("c").build_counter().unwrap();
        counter.sync().unwrap();

        let after = drain!(s);
        assert!(
            extract_counter!(after, "qortoo_sync_total", "result", "success") >= 1,
            "sync_total[success] should be recorded"
        );
    }

    #[test]
    #[serial_test::serial]
    #[instrument]
    fn can_record_sync_failure_and_backoff() {
        let s = snapshotter();
        let (collection, key, resource_id) = get_test_ids!();
        drain!(s);

        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client = Client::builder(collection, "client")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key).build_counter().unwrap();

        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &client.get_cuid())
            .unwrap();
        interceptor.set_after_pull(|_| {
            Err(
                ClientPushPullError::FailedInConnectivity(ConnectivityError::ResourceNotFound(
                    "injected".into(),
                ))
                .mapping(),
            )
        });

        let _ = counter.sync();

        let after = drain!(s);
        assert!(
            extract_counter!(after, "qortoo_sync_total", "result", "failure") >= 1,
            "sync_total[failure] should be recorded"
        );
        assert!(
            extract_counter!(after, "qortoo_backoff_total") >= 1,
            "backoff_total should be recorded"
        );
    }
}
