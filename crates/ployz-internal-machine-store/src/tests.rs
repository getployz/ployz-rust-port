use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ployz_internal_machine_api_pb::{Ip, MachineInfo, NetworkConfig};

use super::*;

type TestSubscription = Result<(Vec<DbRow>, VecDeque<Result<(), String>>), String>;

#[derive(Default)]
struct TestBackend {
    calls: Mutex<Vec<String>>,
    exec_results: Mutex<VecDeque<Result<u64, String>>>,
    query_results: Mutex<VecDeque<Result<Vec<DbRow>, String>>>,
    query_terminal_errors: Mutex<VecDeque<Option<String>>>,
    subscribe_results: Mutex<VecDeque<TestSubscription>>,
}

impl TestBackend {
    fn call_log(&self) -> Vec<String> {
        self.calls.lock().expect("calls lock").clone()
    }

    fn record(&self, kind: &str, query: &str, params: &Option<Vec<SqlValue>>) {
        self.calls
            .lock()
            .expect("calls lock")
            .push(format!("{kind}: {query} | {params:?}"));
    }
}

impl Backend for TestBackend {
    fn exec<'a>(
        &'a self,
        query: &'a str,
        params: Option<Vec<SqlValue>>,
    ) -> BoxFuture<'a, Result<u64, BoxError>> {
        self.record("exec", query, &params);
        let result = self
            .exec_results
            .lock()
            .expect("exec results lock")
            .pop_front()
            .unwrap_or(Ok(1));
        Box::pin(async move { result.map_err(test_error) })
    }

    fn query<'a>(
        &'a self,
        query: &'a str,
        params: Option<Vec<SqlValue>>,
        _columns: &'a [ColumnKind],
    ) -> BoxFuture<'a, Result<QueryRows, BoxError>> {
        self.record("query", query, &params);
        let result = self
            .query_results
            .lock()
            .expect("query results lock")
            .pop_front()
            .unwrap_or(Ok(Vec::new()));
        let terminal_error = self
            .query_terminal_errors
            .lock()
            .expect("query terminal errors lock")
            .pop_front()
            .flatten()
            .map(test_error);
        Box::pin(async move {
            result
                .map(|rows| QueryRows {
                    rows,
                    terminal_error,
                })
                .map_err(test_error)
        })
    }

    fn subscribe<'a>(
        &'a self,
        query: &'a str,
        params: Option<Vec<SqlValue>>,
        _columns: &'a [ColumnKind],
    ) -> BoxFuture<'a, Result<(Vec<DbRow>, Box<dyn ChangeSource>), BoxError>> {
        self.record("subscribe", query, &params);
        let result = self
            .subscribe_results
            .lock()
            .expect("subscribe results lock")
            .pop_front()
            .unwrap_or(Ok((Vec::new(), VecDeque::new())));
        Box::pin(async move {
            result
                .map(|(rows, events)| {
                    let source: Box<dyn ChangeSource> = Box::new(TestChanges(events));
                    (rows, source)
                })
                .map_err(test_error)
        })
    }
}

struct TestChanges(VecDeque<Result<(), String>>);

impl ChangeSource for TestChanges {
    fn next(&mut self) -> Pin<Box<dyn Future<Output = Option<Result<(), BoxError>>> + '_>> {
        let event = self.0.pop_front();
        Box::pin(async move { event.map(|result| result.map_err(test_error)) })
    }
}

fn test_error(message: String) -> BoxError {
    Box::new(ChainedTestError(SimpleError(message)))
}

#[derive(Debug)]
struct ChainedTestError(SimpleError);

impl fmt::Display for ChainedTestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl StdError for ChainedTestError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.0)
    }
}

fn text(value: &str) -> DbValue {
    DbValue::Text(value.to_owned())
}

fn raw(value: &str) -> DbValue {
    DbValue::Raw(value.to_owned())
}

fn machine(id: &str, name: &str) -> MachineInfo {
    MachineInfo {
        id: id.into(),
        name: name.into(),
        network: Some(NetworkConfig {
            public_key: vec![7; 32],
            ..NetworkConfig::default()
        }),
        public_ip: Some(Ip {
            ip: vec![192, 0, 2, 7],
        }),
        daemon_version: "1.2.3".into(),
        docker_version: "28.0".into(),
        hostname: "node.example".into(),
        arch: "amd64".into(),
        os_pretty_name: "Ployz OS".into(),
        kernel_version: "6.8".into(),
    }
}

#[test]
fn schema_retains_generated_columns_and_indexes() {
    assert!(SCHEMA.contains("value      ANY"));
    assert!(SCHEMA.contains("name       TEXT AS (json_extract(info, '$.name'))"));
    assert!(SCHEMA.contains(
        "service_id   TEXT AS (json_extract(container, '$.Config.Labels.\"uncloud.service.id\"'))"
    ));
    assert!(SCHEMA.contains("CREATE INDEX idx_containers_service_name"));
}

#[test]
fn store_timestamp_matches_go_time_date_time_validation() {
    assert_eq!(
        StoreTimestamp::parse("2024-02-29 23:59:59")
            .expect("leap day")
            .as_str(),
        "2024-02-29 23:59:59"
    );
    for invalid in [
        "2023-02-29 00:00:00",
        "2026-13-01 00:00:00",
        "2026-01-01 24:00:00",
        "2026-01-01T00:00:00",
    ] {
        assert!(
            StoreTimestamp::parse(invalid).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn machine_json_matches_protojson_and_discards_unknown_fields() {
    let encoded = encode_machine(&machine("m1", "alpha")).expect("encode machine");
    assert_eq!(
        encoded,
        r#"{"id":"m1","name":"alpha","network":{"publicKey":"BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc="},"publicIp":{"ip":"wAACBw=="},"daemonVersion":"1.2.3","dockerVersion":"28.0","hostname":"node.example","arch":"amd64","osPrettyName":"Ployz OS","kernelVersion":"6.8"}"#
    );

    let decoded = decode_machine(
        r#"{"id":"m1","name":"alpha","network":{"public_key":"BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwf=","endpoints":null,"future":true},"public_ip":{"ip":"wAACCw=="},"unknown":"ignored"}"#,
    )
    .expect("decode machine");
    assert_eq!(decoded.id, "m1");
    assert_eq!(decoded.network.expect("network").public_key, vec![7; 32]);
    assert_eq!(
        decoded.public_ip.expect("public IP").ip,
        vec![192, 0, 2, 11]
    );

    let accepted_base64 =
        decode_machine(r#"{"network":{"publicKey":"/x=="},"public_ip":{"ip":"_w"}}"#)
            .expect("Go protojson Base64 forms");
    assert_eq!(accepted_base64.network.expect("network").public_key, [255]);
    assert_eq!(accepted_base64.public_ip.expect("public IP").ip, [255]);

    let special = MachineInfo {
        name: "<>&\u{2028}\u{2029}".into(),
        ..MachineInfo::default()
    };
    assert_eq!(
        encode_machine(&special).expect("protobuf JSON special characters"),
        "{\"name\":\"<>&\u{2028}\u{2029}\"}"
    );
}

#[test]
fn machine_json_rejects_duplicate_protobuf_field_aliases() {
    for json in [
        r#"{"id":"first","id":"second"}"#,
        r#"{"daemonVersion":"one","daemon_version":"two"}"#,
        r#"{"network":{"publicKey":"Bw==","public_key":"CA=="}}"#,
        r#"{"network":{"endpoints":[{"port":1,"port":"2"}]}}"#,
    ] {
        assert!(
            decode_machine(json)
                .expect_err("duplicate alias was accepted")
                .contains("duplicate field"),
            "fixture: {json}"
        );
    }

    let defaults = decode_machine(
        r#"{"id":null,"network":{"publicKey":null,"endpoints":null},"future":{"id":"ignored"}}"#,
    )
    .expect("protobuf null/default and unknown-field behavior");
    assert!(defaults.id.is_empty());
    assert_eq!(
        defaults.network.expect("network").public_key,
        Vec::<u8>::new()
    );
}

#[tokio::test]
async fn cluster_values_preserve_not_found_and_sql_types() {
    let backend = Arc::new(TestBackend::default());
    backend
        .query_results
        .lock()
        .expect("query results")
        .extend([
            Ok(Vec::new()),
            Ok(vec![vec![raw(r#""AP8=""#)]]),
            Ok(vec![vec![raw(r#""hello""#)]]),
        ]);
    let store = Store::with_backend(backend.clone());

    assert!(matches!(
        store.get::<String>("missing").await,
        Err(Error::KeyNotFound)
    ));
    assert_eq!(store.get::<Vec<u8>>("blob").await.expect("blob"), [0, 255]);
    assert_eq!(store.get::<String>("text").await.expect("text"), "hello");
    store.put("key", [0, 255].as_slice()).await.expect("put");
    store.delete("key").await.expect("delete");

    let calls = backend.call_log().join("\n");
    assert!(calls.contains("SELECT value FROM cluster WHERE key = ?"));
    assert!(calls.contains("INSERT OR REPLACE INTO cluster"));
    assert!(calls.contains("Bytes([0, 255])"));
    assert!(calls.contains("DELETE FROM cluster WHERE key = ?"));
}

#[tokio::test]
async fn machine_crud_validates_identity_network_and_affected_rows() {
    let backend = Arc::new(TestBackend::default());
    let valid_json = encode_machine(&machine("m1", "alpha")).expect("machine JSON");
    backend
        .query_results
        .lock()
        .expect("query results")
        .extend([
            Ok(vec![vec![text(&valid_json)]]),
            Ok(vec![vec![text(&valid_json.replace("\"m1\"", "\"other\""))]]),
        ]);
    backend
        .exec_results
        .lock()
        .expect("exec results")
        .extend([Ok(1), Ok(0), Ok(0)]);
    let store = Store::with_backend(backend);

    assert_eq!(
        store.get_machine("m1").await.expect("get machine").name,
        "alpha"
    );
    assert!(
        matches!(store.get_machine("m1").await, Err(Error::InvalidData(message)) if message.contains("machine ID mismatch"))
    );
    assert!(matches!(
        store.get_machine("").await,
        Err(Error::InvalidInput(_))
    ));
    store
        .create_machine(&machine("m1", "alpha"))
        .await
        .expect("create");
    assert!(
        matches!(store.update_machine(&machine("m2", "beta")).await, Err(Error::MachineNotFound(id)) if id == "m2")
    );
    assert!(
        matches!(store.delete_machine("m3").await, Err(Error::MachineNotFound(id)) if id == "m3")
    );
}

#[tokio::test]
async fn list_machines_orders_at_sql_boundary_and_skips_partial_or_invalid_rows() {
    let backend = Arc::new(TestBackend::default());
    let valid = encode_machine(&machine("m1", "alpha")).expect("valid JSON");
    let invalid = r#"{"id":"bad","network":{"publicKey":"AQ=="}}"#;
    backend
        .query_results
        .lock()
        .expect("query results")
        .push_back(Ok(vec![
            vec![text("partial"), text("{}")],
            vec![text("bad"), text(invalid)],
            vec![text("m1"), text(&valid)],
        ]));
    let store = Store::with_backend(backend.clone());
    let machines = store.list_machines().await.expect("list machines");
    assert_eq!(
        machines
            .iter()
            .map(|machine| machine.id.as_str())
            .collect::<Vec<_>>(),
        ["m1"]
    );
    assert!(backend.call_log()[0].contains("ORDER BY name"));
}

#[tokio::test]
async fn row_stream_failures_preserve_accumulated_results_but_get_reports_empty_failure() {
    let backend = Arc::new(TestBackend::default());
    let valid = encode_machine(&machine("m1", "alpha")).expect("valid JSON");
    backend
        .query_results
        .lock()
        .expect("query results")
        .extend([
            Ok(vec![vec![text("m1"), text(&valid)]]),
            Ok(Vec::new()),
            Ok(Vec::new()),
        ]);
    backend
        .query_terminal_errors
        .lock()
        .expect("query terminal errors")
        .extend([
            Some("stream failed after row".into()),
            Some("stream failed before row".into()),
            Some("machine stream failed before row".into()),
        ]);
    let store = Store::with_backend(backend);

    let machines = store.list_machines().await.expect("partial machine list");
    assert_eq!(machines.len(), 1);
    assert_eq!(machines[0].id, "m1");

    let error = store
        .get::<String>("key")
        .await
        .expect_err("empty failed query must not become key-not-found");
    assert!(error.to_string().contains("stream failed before row"));
    assert!(error.source().is_some());

    let error = store
        .get_machine("m2")
        .await
        .expect_err("failed machine query must not become machine-not-found");
    assert!(matches!(error, Error::Operation { .. }));
    assert!(
        error
            .to_string()
            .contains("machine stream failed before row")
    );
    assert!(error.source().is_some());
}

#[tokio::test]
async fn versions_and_known_gaps_retain_actor_formats() {
    let backend = Arc::new(TestBackend::default());
    backend
        .query_results
        .lock()
        .expect("query results")
        .extend([
            Ok(vec![vec![
                DbValue::Bytes((0_u8..16).collect()),
                DbValue::Integer(42),
            ]]),
            Ok(vec![vec![
                DbValue::Bytes(vec![0xab, 0xcd]),
                DbValue::Integer(2),
                DbValue::Integer(5),
            ]]),
        ]);
    let store = Store::with_backend(backend);
    assert_eq!(
        store
            .version()
            .await
            .expect("version")
            .get("00010203-0405-0607-0809-0a0b0c0d0e0f"),
        Some(&42)
    );
    assert_eq!(
        store.known_missing_changes().await.expect("gaps"),
        [MissingChange {
            actor_id: "abcd".into(),
            start_version: 2,
            end_version: 5
        }]
    );
}

#[tokio::test]
async fn container_upsert_removes_secrets_and_stabilises_mount_order() {
    let backend = Arc::new(TestBackend::default());
    let store = Store::with_backend(backend.clone());
    let container = ServiceContainer::from_json(
        br#"{"Id":"ctr","Created":"2026-01-01T00:00:00Z","Path":"/bin/app","Args":[],"Image":"sha256:1","Name":"/app","Config":{"Env":["SECRET=one"],"Labels":{"uncloud.service.id":"svc"}},"Mounts":[{"Source":"/z","Destination":"/data"},{"Source":"/a","Destination":"/data"}],"ServiceSpec":{"Container":{"Env":{"SECRET":"two"}}}}"#,
    )
    .expect("container fixture");
    assert_eq!(container.container.name, "app");
    store
        .create_or_update_container(container, "m1")
        .await
        .expect("upsert");
    let call = &backend.call_log()[0];
    assert!(!call.contains("SECRET=one"));
    assert!(!call.contains("SECRET\\\":\\\"two"));
    assert!(
        call.find("\\\"Source\\\":\\\"/a").expect("mount a")
            < call.find("\\\"Source\\\":\\\"/z").expect("mount z")
    );
    assert!(call.contains("synced"));
}

#[tokio::test]
async fn container_filters_preserve_or_and_delete_and_semantics() {
    let backend = Arc::new(TestBackend::default());
    let store = Store::with_backend(backend.clone());
    store
        .list_containers(&ListOptions {
            machine_ids: vec!["m1".into(), "m2".into()],
            service_id_or_name: ServiceIdOrNameOptions {
                id: "id".into(),
                name: "web".into(),
            },
        })
        .await
        .expect("list");
    store
        .delete_containers(&DeleteOptions {
            ids: vec!["c1".into(), "c2".into()],
            machine_ids: vec!["m1".into()],
        })
        .await
        .expect("delete");
    store
        .delete_containers(&DeleteOptions::default())
        .await
        .expect("delete all");

    let calls = backend.call_log();
    assert!(calls[0].contains("c.machine_id IN (?, ?)"));
    assert!(calls[0].contains("(c.service_id = ? OR c.service_name = ?)"));
    assert!(calls[1].contains("id IN (?, ?) AND machine_id IN (?)"));
    assert!(calls[2].contains("exec: DELETE FROM containers | None"));
}

#[tokio::test]
async fn container_rows_skip_partial_replication_and_parse_timestamp() {
    let backend = Arc::new(TestBackend::default());
    let json = r#"{"Id":"ctr","Created":"2026-01-01T00:00:00Z","Path":"/bin/app","Args":[],"Image":"sha256:1","Name":"/app"}"#;
    backend
        .query_results
        .lock()
        .expect("query results")
        .push_back(Ok(vec![
            vec![
                text("partial"),
                text("{}"),
                text("m1"),
                text("synced"),
                text("2026-01-01 00:00:00"),
            ],
            vec![
                text("ctr"),
                text(json),
                text("m1"),
                text("synced"),
                text("2026-01-02 03:04:05"),
            ],
        ]));
    let store = Store::with_backend(backend);
    let records = store
        .list_containers(&ListOptions::default())
        .await
        .expect("records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].container.container.name, "app");
    assert_eq!(records[0].updated_at.as_str(), "2026-01-02 03:04:05");
}

#[tokio::test]
async fn subscriptions_return_snapshot_signal_changes_and_close_on_error() {
    let backend = Arc::new(TestBackend::default());
    let valid = encode_machine(&machine("m1", "alpha")).expect("valid JSON");
    backend
        .subscribe_results
        .lock()
        .expect("subscribe results")
        .push_back(Ok((
            vec![vec![text("m1"), text(&valid)]],
            VecDeque::from([Ok(()), Err("stream failed".into()), Ok(())]),
        )));
    let store = Store::with_backend(backend);
    let (machines, mut changes) = store.subscribe_machines().await.expect("subscribe");
    assert_eq!(machines[0].id, "m1");
    fn assert_send<T: Send>(future: T) -> T {
        future
    }
    assert_eq!(assert_send(changes.recv()).await, Some(()));
    assert_eq!(changes.recv().await, None);
    let error = changes.error().expect("subscription error");
    assert_eq!(error.to_string(), "stream failed");
    assert_eq!(
        error.source().expect("preserved source").to_string(),
        "stream failed"
    );
    assert_eq!(changes.recv().await, None);
}

struct PendingChanges {
    dropped: Arc<AtomicBool>,
}

impl ChangeSource for PendingChanges {
    fn next(&mut self) -> ChangeFuture<'_> {
        Box::pin(async move {
            let not_send = Rc::new(());
            let _keep_alive = &not_send;
            std::future::pending().await
        })
    }
}

impl Drop for PendingChanges {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn dropping_notifications_cancels_and_reaps_non_send_source() {
    let dropped = Arc::new(AtomicBool::new(false));
    let changes = ChangeNotifications::new(Box::new(PendingChanges {
        dropped: dropped.clone(),
    }));
    tokio::task::yield_now().await;
    drop(changes);

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("notification worker was orphaned");
}
