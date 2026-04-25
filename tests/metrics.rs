mod tests_metrics {
    use std::sync::OnceLock;

    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
    use qortoo::{Client, Datatype};
    use serial_test::serial;
    use tracing::instrument;

    static SNAPSHOTTER: OnceLock<Snapshotter> = OnceLock::new();

    fn snapshotter() -> &'static Snapshotter {
        SNAPSHOTTER.get_or_init(|| {
            let recorder = DebuggingRecorder::new();
            let snapshotter = recorder.snapshotter();
            recorder.install().unwrap();
            snapshotter
        })
    }

    macro_rules! counter_val {
        ($s:expr, $name:expr, $lk:expr, $lv:expr) => {{
            $s.snapshot()
                .into_vec()
                .into_iter()
                .filter(|(ck, _, _, _)| {
                    ck.key().name() == $name
                        && ck
                            .key()
                            .labels()
                            .any(|l| l.key() == $lk && l.value() == $lv)
                })
                .map(|(_, _, _, v)| match v {
                    DebugValue::Counter(n) => n,
                    _ => 0,
                })
                .sum::<u64>()
        }};
    }

    macro_rules! histogram_samples {
        ($snap_vec:expr, $name:expr) => {{
            $snap_vec
                .into_iter()
                .filter(|(ck, _, _, _)| ck.key().name() == $name)
                .map(|(_, _, _, v)| match v {
                    DebugValue::Histogram(xs) => xs.len(),
                    _ => 0,
                })
                .sum::<usize>()
        }};
    }

    #[test]
    #[serial]
    #[instrument]
    fn can_record_sync_success_metrics() {
        let s = snapshotter();
        let base_ok = counter_val!(s, "qortoo_sync_total", "result", "success");

        let client = Client::builder("metrics-test", "can_record_sync_success_metrics")
            .build()
            .unwrap();
        let counter = client.create_datatype("c1").build_counter().unwrap();
        counter.sync().unwrap();

        let after = s.snapshot().into_vec();
        let after_ok: u64 = after
            .iter()
            .filter(|(ck, _, _, _)| {
                ck.key().name() == "qortoo_sync_total"
                    && ck
                        .key()
                        .labels()
                        .any(|l| l.key() == "result" && l.value() == "success")
            })
            .map(|(_, _, _, v)| match v {
                DebugValue::Counter(n) => *n,
                _ => 0,
            })
            .sum();
        let after_fail: u64 = after
            .iter()
            .filter(|(ck, _, _, _)| {
                ck.key().name() == "qortoo_sync_total"
                    && ck
                        .key()
                        .labels()
                        .any(|l| l.key() == "result" && l.value() == "failure")
            })
            .map(|(_, _, _, v)| match v {
                DebugValue::Counter(n) => *n,
                _ => 0,
            })
            .sum();
        let dur_samples = histogram_samples!(after, "qortoo_sync_duration_seconds");

        assert!(after_ok > base_ok, "sync_total[success] should increase");
        assert!(
            dur_samples >= 1,
            "sync_duration_seconds should have at least one sample"
        );
        assert_eq!(
            after_fail, 0,
            "NullConnectivity always succeeds — no failure metric expected"
        );
    }
}
