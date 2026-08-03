use crate::datatypes::common::Attribute;

// --- Metric name constants (Prometheus naming convention) ---

const SYNC_TOTAL: &str = "qortoo_sync_total";
pub(crate) const SYNC_DURATION_SECONDS: &str = "qortoo_sync_duration_seconds";
const TRANSACTIONS_TOTAL: &str = "qortoo_transactions_total";
const BACKOFF_TOTAL: &str = "qortoo_backoff_total";

// --- Label key constants ---

const LABEL_COLLECTION: &str = "collection";
const LABEL_TYPE: &str = "type";
const LABEL_RESULT: &str = "result";
const LABEL_PUSH_PULL: &str = "push_pull";

// --- Label value constants ---

const RESULT_SUCCESS: &str = "success";
const RESULT_FAILURE: &str = "failure";
const PUSH_PULL_PUSH: &str = "push";
const PUSH_PULL_PULL: &str = "pull";

// --- Helper functions ---

pub fn emit_sync(attr: &Attribute, success: bool, duration: std::time::Duration) {
    let result = if success {
        RESULT_SUCCESS
    } else {
        RESULT_FAILURE
    };
    let col = attr.client_common.collection.to_string();
    let dt = attr.r#type.to_string();
    metrics::counter!(
        SYNC_TOTAL,
        LABEL_COLLECTION => col.clone(),
        LABEL_TYPE => dt.clone(),
        LABEL_RESULT => result,
    )
    .increment(1);
    metrics::histogram!(
        SYNC_DURATION_SECONDS,
        LABEL_COLLECTION => col,
        LABEL_TYPE => dt,
        LABEL_RESULT => result,
    )
    .record(duration.as_secs_f64());
}

pub fn emit_backoff(attr: &Attribute) {
    metrics::counter!(
        BACKOFF_TOTAL,
        LABEL_COLLECTION => attr.client_common.collection.to_string(),
        LABEL_TYPE => attr.r#type.to_string(),
    )
    .increment(1);
}

pub fn emit_pushed_transactions(attr: &Attribute, count: usize) {
    emit_transactions(attr, PUSH_PULL_PUSH, count);
}

pub fn emit_pulled_transactions(attr: &Attribute, count: usize) {
    emit_transactions(attr, PUSH_PULL_PULL, count);
}

fn emit_transactions(attr: &Attribute, push_pull: &'static str, count: usize) {
    metrics::counter!(
        TRANSACTIONS_TOTAL,
        LABEL_COLLECTION => attr.client_common.collection.to_string(),
        LABEL_TYPE => attr.r#type.to_string(),
        LABEL_PUSH_PULL => push_pull,
    )
    .increment(count as u64);
}

#[cfg(test)]
mod tests_metrics {
    use std::sync::OnceLock;

    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
    use tracing::instrument;

    use super::{
        LABEL_COLLECTION, LABEL_PUSH_PULL, PUSH_PULL_PULL, PUSH_PULL_PUSH, TRANSACTIONS_TOTAL,
    };
    use crate::{
        Client, DatatypeError,
        connectivity::local_connectivity::LocalConnectivity,
        datatypes::datatype::Datatype,
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

        let collection = get_test_collection_name!();
        let client = Client::builder(collection.clone(), get_test_func_name!())
            .build()
            .unwrap();
        for key in ["c1", "c2"] {
            client
                .create_datatype(key)
                .build_counter()
                .unwrap()
                .sync()
                .unwrap();
        }

        let after = drain!(s);
        assert!(
            after
                .iter()
                .filter(|(ck, _, _, _)| ck.key().name().starts_with("qortoo_"))
                .all(|(ck, _, _, _)| ck.key().labels().all(|label| label.key() != "key")),
            "Qortoo metrics must not expose datatype keys as labels"
        );
        let success_series: Vec<_> = after
            .iter()
            .filter(|(ck, _, _, _)| {
                ck.key().name() == "qortoo_sync_total"
                    && ck
                        .key()
                        .labels()
                        .any(|label| label.key() == "collection" && label.value() == collection)
                    && ck
                        .key()
                        .labels()
                        .any(|label| label.key() == "result" && label.value() == "success")
            })
            .collect();
        assert_eq!(
            success_series.len(),
            1,
            "different datatype keys must aggregate into one metric series"
        );
        assert!(
            matches!(success_series[0].3, DebugValue::Counter(value) if value >= 2),
            "the aggregated series must include both datatype syncs"
        );
        assert!(
            after.iter().any(|(ck, _, _, _)| {
                ck.key().name() == "qortoo_sync_duration_seconds"
                    && ck
                        .key()
                        .labels()
                        .any(|label| label.key() == "result" && label.value() == "success")
            }),
            "sync_duration_seconds[success] should be recorded"
        );
        assert!(
            extract_counter!(after, "qortoo_sync_total", "result", "success") >= 1,
            "sync_total[success] should be recorded"
        );
    }

    #[test]
    #[serial_test::serial]
    #[instrument]
    fn can_record_pushed_and_pulled_transaction_counts() {
        let s = snapshotter();
        drain!(s);

        let collection = get_test_collection_name!();
        let key = get_test_func_name!();
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(collection.clone(), "client2")
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

        counter1.increase().unwrap();
        counter1.increase().unwrap();
        counter1.sync().unwrap();
        counter2.sync().unwrap();
        assert_eq!(counter2.get_value(), 2);

        let after = drain!(s);
        let count_for_push_pull = |push_pull| {
            after
                .iter()
                .filter(|(ck, _, _, _)| {
                    ck.key().name() == TRANSACTIONS_TOTAL
                        && ck.key().labels().any(|label| {
                            label.key() == LABEL_COLLECTION && label.value() == collection
                        })
                        && ck.key().labels().any(|label| {
                            label.key() == LABEL_PUSH_PULL && label.value() == push_pull
                        })
                })
                .map(|(_, _, _, value)| match value {
                    DebugValue::Counter(value) => *value,
                    _ => 0,
                })
                .sum::<u64>()
        };

        assert_eq!(count_for_push_pull(PUSH_PULL_PUSH), 2);
        assert_eq!(count_for_push_pull(PUSH_PULL_PULL), 2);
        assert!(
            after
                .iter()
                .filter(|(ck, _, _, _)| ck.key().name() == TRANSACTIONS_TOTAL)
                .all(|(ck, _, _, _)| ck.key().labels().all(|label| label.key() != "key")),
            "transaction metrics must not expose datatype keys as labels"
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
        interceptor.set_after_pull(|_| Err(DatatypeError::SyncFailed("injected".into()).mapping()));

        let _ = counter.sync();

        let after = drain!(s);
        assert!(
            after
                .iter()
                .filter(|(ck, _, _, _)| ck.key().name().starts_with("qortoo_"))
                .all(|(ck, _, _, _)| ck.key().labels().all(|label| label.key() != "key")),
            "Qortoo metrics must not expose datatype keys as labels"
        );
        assert!(
            extract_counter!(after, "qortoo_sync_total", "result", "failure") >= 1,
            "sync_total[failure] should be recorded"
        );
        assert!(
            after.iter().any(|(ck, _, _, _)| {
                ck.key().name() == "qortoo_sync_duration_seconds"
                    && ck
                        .key()
                        .labels()
                        .any(|label| label.key() == "result" && label.value() == "failure")
            }),
            "sync_duration_seconds[failure] should be recorded"
        );
        assert!(
            extract_counter!(after, "qortoo_backoff_total") >= 1,
            "backoff_total should be recorded"
        );
    }
}
