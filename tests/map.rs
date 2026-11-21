use std::sync::{Arc, mpsc};

use qortoo::{
    Client, Datatype, DatatypeError, DatatypeHandler, DatatypeSet, DatatypeState,
    LocalConnectivity, Map, SnapshotCodecKind, SnapshotCompression, SnapshotWriteOptions,
};
use serde_json::{Value, json};

fn manual_connectivity() -> Arc<LocalConnectivity> {
    #[cfg(feature = "snapshot-bitcode")]
    let codec = SnapshotCodecKind::Bitcode;
    #[cfg(not(feature = "snapshot-bitcode"))]
    let codec = SnapshotCodecKind::Postcard;
    let connectivity = LocalConnectivity::new_arc_with_snapshot_options(SnapshotWriteOptions {
        codec,
        compression: SnapshotCompression::None,
    });
    connectivity.set_realtime(false);
    connectivity
}

#[test]
fn can_use_map_crud_and_distinguish_null_from_absence() {
    let client = Client::builder("test-map", "crud")
        .build_and_open()
        .unwrap();
    let map = client.create_datatype("settings").build_map().unwrap();
    assert!(map.is_empty());
    assert_eq!(map.get::<Value>("").unwrap(), None);

    let raw = b" { \"number\": 9007199254740993 } ";
    assert_eq!(map.put_raw("other", raw).unwrap(), None);
    assert_eq!(map.get_raw("other").as_deref(), Some(raw.as_slice()));
    assert_eq!(map.put("", &Value::Null).unwrap(), None);
    assert_eq!(map.get::<Option<String>>("").unwrap(), Some(None));
    assert!(map.contains_key(""));
    assert_eq!(map.len(), 2);
    assert_eq!(
        map.to_json().unwrap(),
        json!({"": null, "other": {"number": 9007199254740993_u64}})
    );
    assert_eq!(map.put("", "dark").unwrap(), Some(Value::Null));
    assert_eq!(map.remove("").unwrap(), Some(json!("dark")));
    assert_eq!(map.remove("").unwrap(), None);
    assert_eq!(
        map.remove_raw("other").unwrap().as_deref(),
        Some(raw.as_slice())
    );
    assert!(!map.contains_key(""));
    assert!(map.is_empty());
}

#[test]
fn can_preserve_map_values_and_prior_writes_when_conversion_fails() {
    let client = Client::builder("test-map", "conversion")
        .build_and_open()
        .unwrap();
    let map = client.create_datatype("settings").build_map().unwrap();
    map.put_raw("wide", b"1e400").unwrap();
    assert!(matches!(
        map.to_json(),
        Err(DatatypeError::ValueConversion(_))
    ));

    map.transaction("commit valid writes", |map| {
        map.put("keep", &true)?;
        let version = map.get_client_version();
        for result in [map.put("wide", &0), map.remove("wide")] {
            assert!(matches!(result, Err(DatatypeError::ValueConversion(_))));
        }
        assert!(matches!(
            map.put_raw("bad", b"\"\xff\""),
            Err(DatatypeError::ValueConversion(_))
        ));
        assert_eq!(map.get_client_version(), version);
        Ok(())
    })
    .unwrap();
    assert_eq!(map.get::<bool>("keep").unwrap(), Some(true));
    assert_eq!(map.get_raw("wide").as_deref(), Some(b"1e400".as_slice()));
    assert!(!map.contains_key("bad"));
    assert_eq!(
        map.remove_raw("wide").unwrap().as_deref(),
        Some(b"1e400".as_slice())
    );

    let result = map.transaction("rollback writes", |map| {
        map.remove("keep")?;
        map.put("new", &42)?;
        map.put_raw("bad", b"null true")?;
        Ok(())
    });
    assert!(matches!(result, Err(DatatypeError::TransactionFailed(_))));
    assert_eq!(map.to_json().unwrap(), json!({"keep": true}));
}

#[test]
fn can_read_map_logical_state_size_with_pending_writes_and_tombstones() {
    let client = Client::builder("test-map", "state-size")
        .with_connectivity(manual_connectivity())
        .build_and_open()
        .unwrap();
    let map = client.create_datatype("settings").build_map().unwrap();
    assert_eq!(map.get_state_bytes(), 0);

    map.put_raw("한", b" null ").unwrap();
    assert_eq!(map.get_state_bytes(), 51);
    assert_eq!(map.get_synced_client_version(), 0);

    let result = map.transaction("rollback size changes", |map| {
        map.remove_raw("한")?;
        map.put_raw("z", b"true")?;
        assert_eq!(map.get_state_bytes(), 92);
        map.put_raw("invalid", b"null true")?;
        Ok(())
    });
    assert!(matches!(result, Err(DatatypeError::TransactionFailed(_))));
    assert_eq!(map.get_state_bytes(), 51);
    map.remove_raw("한").unwrap();
    assert!(map.is_empty());
    assert_eq!(map.get_state_bytes(), 45);
    assert_eq!(
        client.get_datatype("settings").unwrap().get_state_bytes(),
        45
    );
}

#[test]
fn can_sync_map_through_all_builders_and_resolve_its_handler() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Map>();

    let connectivity = manual_connectivity();
    let client = |alias| {
        Client::builder("test-map", alias)
            .with_connectivity(connectivity.clone())
            .build_and_open()
            .unwrap()
    };

    let (send, receive) = mpsc::channel();
    let handler = DatatypeHandler::new().set_on_state_change(move |datatype, _, state| {
        if state == DatatypeState::Subscribed {
            send.send(matches!(datatype, DatatypeSet::Map(_))).unwrap();
        }
    });
    let creator = client("creator");
    let map = creator
        .create_datatype("settings")
        .with_handler(0, handler)
        .build_map()
        .unwrap();
    map.put("color", "blue").unwrap();
    map.sync().unwrap();
    assert!(
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
    );

    let subscriber = client("subscriber");
    let subscribed = subscriber
        .subscribe_datatype("settings")
        .build_map()
        .unwrap();
    subscribed.sync().unwrap();
    assert_eq!(
        subscribed.get::<String>("color").unwrap().as_deref(),
        Some("blue")
    );
    subscribed.remove("color").unwrap();
    subscribed.put("count", &42).unwrap();
    subscribed.sync().unwrap();

    let observer = client("observer");
    let observed = observer
        .subscribe_or_create_datatype("settings")
        .build_map()
        .unwrap();
    observed.sync().unwrap();
    map.sync().unwrap();
    assert_eq!(observed.to_json().unwrap(), json!({"count": 42}));
    assert_eq!(observed.to_json().unwrap(), map.to_json().unwrap());
}
