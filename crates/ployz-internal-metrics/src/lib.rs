//! Process-wide Prometheus metrics used by Ployz.

use std::collections::{HashMap, hash_map::Entry};
use std::fmt::Debug;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use prometheus::{
    Gauge, GaugeVec, IntCounter, IntCounterVec, Opts, Registry,
    core::{Collector, Desc},
    proto::{Metric, MetricFamily},
};
use protobuf::{Message, well_known_types::timestamp::Timestamp};

/// Namespace applied to every Ployz application metric.
pub const NAMESPACE: &str = "ployz";

const DAEMON_SUBSYSTEM: &str = "ployzd";

/// A counter vector whose collected children include their creation timestamp.
///
/// Children remain ordinary [`IntCounter`] handles. Register this vector as a
/// collector and gather it through the registry to receive Prometheus
/// `Counter.created_timestamp` field 3 for every created child.
#[derive(Clone)]
pub struct CreatedIntCounterVec {
    inner: IntCounterVec,
    variable_labels: Arc<[String]>,
    created_at: Arc<Mutex<HashMap<Vec<String>, SystemTime>>>,
}

impl CreatedIntCounterVec {
    /// Creates a counter vector that records each label set's first access.
    pub fn new(opts: Opts, variable_labels: &[&str]) -> prometheus::Result<Self> {
        Ok(Self {
            inner: IntCounterVec::new(opts, variable_labels)?,
            variable_labels: variable_labels
                .iter()
                .map(|label| (*label).to_owned())
                .collect::<Vec<_>>()
                .into(),
            created_at: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Returns the child for `values`, recording the winning first access.
    pub fn get_metric_with_label_values<V>(&self, values: &[V]) -> prometheus::Result<IntCounter>
    where
        V: AsRef<str> + Debug,
    {
        let key = values
            .iter()
            .map(|value| value.as_ref().to_owned())
            .collect::<Vec<_>>();
        let mut created_at = self
            .created_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        match created_at.entry(key) {
            Entry::Occupied(_) => self.inner.get_metric_with_label_values(values),
            Entry::Vacant(entry) => {
                let child = self.inner.get_metric_with_label_values(values)?;
                entry.insert(SystemTime::now());
                Ok(child)
            }
        }
    }

    /// Returns the child for `values`, panicking on inconsistent cardinality.
    pub fn with_label_values<V>(&self, values: &[V]) -> IntCounter
    where
        V: AsRef<str> + Debug,
    {
        self.get_metric_with_label_values(values).unwrap()
    }

    fn metric_key(&self, metric: &Metric) -> Option<Vec<String>> {
        self.variable_labels
            .iter()
            .map(|name| {
                metric
                    .label
                    .iter()
                    .find(|label| label.name() == name)
                    .map(|label| label.value().to_owned())
            })
            .collect()
    }
}

impl Collector for CreatedIntCounterVec {
    fn desc(&self) -> Vec<&Desc> {
        self.inner.desc()
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let created_at = self
            .created_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut families = self.inner.collect();

        for family in &mut families {
            for metric in &mut family.metric {
                let Some(created) = self.metric_key(metric).and_then(|key| created_at.get(&key))
                else {
                    continue;
                };
                let Some(counter) = metric.counter.as_mut() else {
                    continue;
                };
                let timestamp = Timestamp::from(*created)
                    .write_to_bytes()
                    .expect("serialize generated Timestamp");
                counter
                    .mut_unknown_fields()
                    .add_length_delimited(3, timestamp);
            }
        }

        families
    }
}

/// Registered handles for all process metrics.
pub struct Metrics {
    build_info: GaugeVec,
    dns_queries: CreatedIntCounterVec,
}

impl Metrics {
    fn register(registry: &Registry) -> prometheus::Result<Self> {
        let build_info = GaugeVec::new(
            Opts::new("build_info", "Build information.")
                .namespace(NAMESPACE)
                .subsystem(DAEMON_SUBSYSTEM),
            &["version"],
        )?;
        let dns_queries = CreatedIntCounterVec::new(
            Opts::new("query_total", "Counter of DNS queries.")
                .namespace(NAMESPACE)
                .subsystem("dns"),
            &["internal", "status"],
        )?;

        registry.register(Box::new(build_info.clone()))?;
        registry.register(Box::new(dns_queries.clone()))?;

        Ok(Self {
            build_info,
            dns_queries,
        })
    }

    /// Returns the build-info gauge for `version`, creating it on first access.
    pub fn build_info(&self, version: &str) -> Gauge {
        self.build_info.with_label_values(&[version])
    }

    /// Returns the DNS-query counter for the exact supplied label values.
    pub fn dns_query(&self, internal: &str, status: &str) -> IntCounter {
        self.dns_queries.with_label_values(&[internal, status])
    }
}

static METRICS: LazyLock<Metrics> = LazyLock::new(|| {
    Metrics::register(prometheus::default_registry())
        .unwrap_or_else(|error| panic!("register Ployz metrics: {error}"))
});

/// Returns the process-wide metric handles.
///
/// The first call registers every collector with Prometheus's default registry.
/// Registration conflicts panic, matching the oracle's automatic registration.
pub fn metrics() -> &'static Metrics {
    &METRICS
}

/// Returns the shared default registry after ensuring all Ployz metrics exist.
pub fn registry() -> &'static Registry {
    LazyLock::force(&METRICS);
    prometheus::default_registry()
}

/// Label value used when an operation completed without an error.
pub const OK: &str = "ok";

/// Label value used when an operation returned an error.
pub const ERR: &str = "err";

/// Returns [`OK`] when `error` is absent and [`ERR`] when it is present.
pub const fn status<E>(error: Option<&E>) -> &'static str {
    if error.is_some() { ERR } else { OK }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::{
        Encoder, PROTOBUF_FORMAT, ProtobufEncoder, Registry, TEXT_FORMAT, TextEncoder,
    };

    fn gather_text(registry: &Registry) -> String {
        let mut output = Vec::new();
        TextEncoder::new()
            .encode(&registry.gather(), &mut output)
            .expect("encode gathered metrics");
        String::from_utf8(output).expect("Prometheus text is UTF-8")
    }

    #[test]
    fn status_maps_absent_and_present_errors() {
        let error = std::io::Error::other("failed");

        assert_eq!(status(None::<&std::io::Error>), OK);
        assert_eq!(status(Some(&error)), ERR);
    }

    #[test]
    fn registers_the_renamed_metric_contract() {
        let registry = Registry::new();
        let metrics = Metrics::register(&registry).expect("register metrics");

        metrics.build_info("v1.2.3").set(1.0);
        metrics.dns_query("false", OK).inc();

        assert_eq!(
            gather_text(&registry),
            concat!(
                "# HELP ployz_dns_query_total Counter of DNS queries.\n",
                "# TYPE ployz_dns_query_total counter\n",
                "ployz_dns_query_total{internal=\"false\",status=\"ok\"} 1\n",
                "# HELP ployz_ployzd_build_info Build information.\n",
                "# TYPE ployz_ployzd_build_info gauge\n",
                "ployz_ployzd_build_info{version=\"v1.2.3\"} 1\n",
            )
        );
    }

    #[test]
    fn repeated_access_reuses_values_and_keeps_labels_exact() {
        let registry = Registry::new();
        let metrics = Metrics::register(&registry).expect("register metrics");

        let first_build = metrics.build_info("v1");
        let same_build = metrics.build_info("v1");
        first_build.set(7.0);
        assert_eq!(same_build.get(), 7.0);
        same_build.set(1.0);
        assert_eq!(first_build.get(), 1.0);

        let first_query = metrics.dns_query("false", OK);
        let same_query = metrics.dns_query("false", OK);
        first_query.inc();
        same_query.inc_by(2);
        metrics.dns_query("False", OK).inc();
        metrics.dns_query("false", "timeout").inc();

        assert_eq!(first_query.get(), 3);
        assert_eq!(metrics.dns_query("False", OK).get(), 1);
        assert_eq!(metrics.dns_query("false", "timeout").get(), 1);
    }

    #[test]
    fn encoders_cover_downstream_scrape_formats() {
        let registry = Registry::new();
        let metrics = Metrics::register(&registry).expect("register metrics");
        metrics.build_info("v1").set(1.0);
        metrics.dns_query("true", OK).inc();
        let families = registry.gather();

        let text_encoder = TextEncoder::new();
        assert_eq!(TEXT_FORMAT, "text/plain; version=0.0.4");
        assert_eq!(text_encoder.format_type(), TEXT_FORMAT);

        let protobuf_encoder = ProtobufEncoder::new();
        assert_eq!(
            PROTOBUF_FORMAT,
            concat!(
                "application/vnd.google.protobuf; ",
                "proto=io.prometheus.client.MetricFamily; ",
                "encoding=delimited"
            )
        );
        assert_eq!(protobuf_encoder.format_type(), PROTOBUF_FORMAT);

        let mut protobuf = Vec::new();
        protobuf_encoder
            .encode(&families, &mut protobuf)
            .expect("encode delimited protobuf metrics");
        assert!(!protobuf.is_empty());
    }

    #[test]
    fn duplicate_registration_is_an_error() {
        let registry = Registry::new();
        Metrics::register(&registry).expect("first registration succeeds");

        let error = match Metrics::register(&registry) {
            Ok(_) => panic!("duplicate registration must fail"),
            Err(error) => error,
        };

        assert!(matches!(error, prometheus::Error::AlreadyReg));
    }

    #[test]
    fn process_metrics_use_the_default_registry() {
        let shared = registry();
        metrics().dns_query("true", OK);

        assert!(std::ptr::eq(shared, prometheus::default_registry()));
        assert!(
            shared
                .gather()
                .iter()
                .any(|family| family.name() == "ployz_dns_query_total")
        );
        assert!(std::ptr::eq(metrics(), &*METRICS));
    }

    #[test]
    fn concurrent_updates_are_not_lost() {
        use std::sync::Arc;

        let registry = Registry::new();
        let metrics = Arc::new(Metrics::register(&registry).expect("register metrics"));
        let workers = (0..8)
            .map(|_| {
                let metrics = Arc::clone(&metrics);
                std::thread::spawn(move || {
                    for _ in 0..10_000 {
                        metrics.dns_query("true", OK).inc();
                        metrics.build_info("concurrent").set(1.0);
                    }
                })
            })
            .collect::<Vec<_>>();

        for worker in workers {
            worker.join().expect("metric worker must not panic");
        }

        assert_eq!(metrics.dns_query("true", OK).get(), 80_000);
        assert_eq!(metrics.build_info("concurrent").get(), 1.0);
    }
}
