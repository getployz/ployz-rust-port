use std::time::SystemTime;

use ployz_internal_metrics::CreatedIntCounterVec;
use prometheus::{Opts, Registry};
use protobuf::{Message, UnknownValueRef, well_known_types::timestamp::Timestamp};

fn created_timestamp(registry: &Registry, label_value: &str) -> SystemTime {
    let families = registry.gather();
    let metric = families[0]
        .metric
        .iter()
        .find(|metric| metric.label[0].value() == label_value)
        .expect("counter child is gathered");
    let field = metric
        .counter
        .as_ref()
        .expect("metric is a counter")
        .special_fields
        .unknown_fields()
        .get(3)
        .expect("counter has created_timestamp field 3");
    let UnknownValueRef::LengthDelimited(bytes) = field else {
        panic!("created_timestamp is a message");
    };

    Timestamp::parse_from_bytes(bytes)
        .expect("created_timestamp is a valid Timestamp")
        .into()
}

#[test]
fn collection_exposes_stable_first_child_creation_timestamps() {
    let registry = Registry::new();
    let counters = CreatedIntCounterVec::new(Opts::new("requests_total", "Requests."), &["status"])
        .expect("create counter vector");
    registry
        .register(Box::new(counters.clone()))
        .expect("register counter vector");

    let before_first = SystemTime::now();
    let first = counters.with_label_values(&["ok"]);
    let after_first = SystemTime::now();
    first.inc();

    let first_created = created_timestamp(&registry, "ok");
    assert!(first_created >= before_first);
    assert!(first_created <= after_first);
    assert_eq!(created_timestamp(&registry, "ok"), first_created);

    let before_second = SystemTime::now();
    counters.with_label_values(&["err"]).inc();
    let after_second = SystemTime::now();
    let second_created = created_timestamp(&registry, "err");
    assert!(second_created >= before_second);
    assert!(second_created <= after_second);
    assert_eq!(created_timestamp(&registry, "err"), second_created);
}

#[test]
fn concurrent_first_access_creates_one_timestamped_child() {
    let registry = Registry::new();
    let counters =
        CreatedIntCounterVec::new(Opts::new("concurrent_total", "Concurrent."), &["kind"])
            .expect("create counter vector");
    registry
        .register(Box::new(counters.clone()))
        .expect("register counter vector");

    let before = SystemTime::now();
    let workers = (0..16)
        .map(|_| {
            let counters = counters.clone();
            std::thread::spawn(move || counters.with_label_values(&["shared"]).inc())
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().expect("counter worker must not panic");
    }
    let after = SystemTime::now();

    let families = registry.gather();
    assert_eq!(families.len(), 1);
    assert_eq!(families[0].metric.len(), 1);
    assert_eq!(families[0].metric[0].get_counter().value(), 16.0);
    let created = created_timestamp(&registry, "shared");
    assert!(created >= before);
    assert!(created <= after);
    assert_eq!(created_timestamp(&registry, "shared"), created);
}
