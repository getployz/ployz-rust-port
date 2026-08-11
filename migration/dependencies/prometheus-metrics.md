# Dependency decision: `prometheus-metrics`

| Field | Value |
| --- | --- |
| Status | `approved` |
| Selected dependency | `prometheus = { version = "=0.14.0", default-features = false, features = ["protobuf"] }` |
| License | `Apache-2.0` |
| Research date | `2026-08-11` UTC |
| Request | Delegated capability request; no on-disk request exists at base `f814fb555dcbc12961c64cfada5ad4a34fb9e576` |

## Required behavior and oracle evidence

The frozen oracle declares two automatically registered vectors in
`upstream/uncloud/internal/metrics/metrics.go:9`: a gauge named from namespace
`uncloud`, subsystem `uncloudd`, name `build_info`, help `Build information.`,
and label `version`; and a counter named from namespace `uncloud`, subsystem
`dns`, name `query_total`, help `Counter of DNS queries.`, and labels
`internal,status`. The daemon sets the build gauge to `1` for its version in
`upstream/uncloud/cmd/uncloudd/main.go:50`. DNS callers increment
`internal="false"` with `status="ok"|"err"` after a forwarded query and
`internal="true",status="ok"` after an internal query, including NXDOMAIN, in
`upstream/uncloud/internal/machine/dns/server.go:191` and
`upstream/uncloud/internal/machine/dns/server.go:237`.

The expanded request also covers `upstream/uncloud/internal/machine/metrics`.
Its server attaches Go's `promhttp.Handler()` to `/metrics`, which gathers the
default/shared registry (`server.go:25-29`). At the frozen Go dependency version,
the default handler negotiates Prometheus text or protobuf from `Accept`, applies
supported response compression from `Accept-Encoding`, reports the first gather
error as HTTP 500, stops on an encode error, and registers scrape-count and
in-flight self-metrics. These defaults are documented in the pinned
[`promhttp` v1.22.0 source](https://github.com/prometheus/client_golang/blob/v1.22.0/prometheus/promhttp/http.go#L84-L106),
including negotiation/error handling and instrumentation. The pinned
[`expfmt` v0.62.0 negotiation source](https://github.com/prometheus/common/blob/v0.62.0/expfmt/encode.go)
defines the exact protobuf variants and text fallback. The current
[Prometheus scrape negotiation specification](https://prometheus.io/docs/instrumenting/content_negotiation/)
defines delimited `io.prometheus.client.MetricFamily` protobuf and Prometheus
text `0.0.4` as scrape protocols.

Therefore the required dependency behavior is:

- registered, labeled gauge and counter vectors with deterministic names, help,
  types, and labels;
- thread-safe set/increment and collection through one shared registry;
- explicit duplicate/descriptor-conflict registration failure;
- Prometheus text `0.0.4` and delimited protobuf collection suitable for a
  scrape endpoint, plus compatibility handling for the legacy protobuf text
  variants accepted by the frozen Go handler, with encoder errors exposed to
  the HTTP layer;
- no dependency-owned network service or async runtime.

## Primary-source evidence

- The exact [`prometheus` 0.14.0 manifest](https://docs.rs/crate/prometheus/0.14.0/source/Cargo.toml.orig)
  declares Apache-2.0, MSRV 1.81, and `protobuf` as the only selected feature.
  `push`, `process`, `nightly`, and protobuf code generation (`gen`) remain off;
  therefore the selected graph has no network client/server, async runtime,
  process collector, FFI-only feature, or `protoc` requirement.
  The probe resolved `protobuf` 3.7.2, which is newer than the fixed versions
  identified by [`RUSTSEC-2024-0437`](https://rustsec.org/advisories/RUSTSEC-2024-0437.html).
- [`Opts`](https://docs.rs/prometheus/0.14.0/prometheus/struct.Opts.html) joins
  namespace, subsystem, and name with underscores and keeps the supplied help
  string. [`GaugeVec`](https://docs.rs/prometheus/0.14.0/prometheus/type.GaugeVec.html),
  [`IntCounterVec`](https://docs.rs/prometheus/0.14.0/prometheus/type.IntCounterVec.html),
  and [`MetricVec`](https://docs.rs/prometheus/0.14.0/prometheus/core/struct.MetricVec.html)
  provide the required label-family operations.
- [`Registry`](https://docs.rs/prometheus/0.14.0/prometheus/struct.Registry.html)
  is `Send + Sync`, rejects repeated or inconsistent descriptors with
  `Error::AlreadyReg`/an error, and gathers families in lexicographic order. The
  crate also exposes a global `default_registry()` and `gather()` for the
  oracle's shared-registry model.
- [`TextEncoder`](https://docs.rs/prometheus/0.14.0/prometheus/struct.TextEncoder.html)
  emits HELP, TYPE, and samples; [`ProtobufEncoder`](https://docs.rs/prometheus/0.14.0/prometheus/struct.ProtobufEncoder.html)
  emits length-delimited `MetricFamily` messages. Both implement the fallible
  [`Encoder`](https://docs.rs/prometheus/0.14.0/prometheus/trait.Encoder.html)
  interface, allowing handler failures to be mapped deliberately.
- The [counter source](https://docs.rs/prometheus/0.14.0/src/prometheus/counter.rs.html)
  uses shared `Arc` values and `AtomicU64` for `IntCounterVec`; the corresponding
  gauge uses atomic storage. The verification probe below exercised concurrent
  mutation and collection.
- The official [crates.io record](https://crates.io/api/v1/crates/prometheus)
  reported 134,000,276 total downloads, 26,827,005 recent downloads, 30,089,999
  downloads of 0.14.0, and a 2025-03-27 release. The official reverse-dependency
  endpoint reported [878 dependent crates](https://crates.io/api/v1/crates/prometheus/reverse_dependencies?page=1&per_page=1).
  The repository had 1.2k stars and 208 forks at research time, and its latest
  commit was the 2025-10-17 [MSRV/dependency maintenance change](https://github.com/tikv/rust-prometheus/commit/81514180fc47ed387d20140119b97c33495f85fe).
  TiKV is a concrete established adopter in its
  [production manifest](https://github.com/tikv/tikv/blob/master/Cargo.toml#L132-L133).

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Registered labeled gauge/counter vectors; exact metadata; concurrent mutation; duplicate failure; shared gathering; scrape encoders and errors | `Opts`, vectors, atomic values, `Registry`, and both encoders directly cover the model. The Rust 1.96 probe reproduced `uncloud_uncloudd_build_info` and `uncloud_dns_query_total`, rejected duplicate registration, and encoded text plus protobuf. The HTTP-specific adaptation is bounded below. | `pass` |
| License and security | Permissive and memory-safe; no unnecessary service/runtime | Apache-2.0. The selected public path is safe Rust. Unsafe/FFI code in optional process, push, and nightly paths is not enabled. `protobuf` uses the crate's pregenerated model and adds no service or runtime. Audit the eventual workspace lock as normal; this decision does not claim transitive dependencies can never acquire advisories. | `pass` |
| Platforms and targets | Linux and supported Rust targets | The enabled APIs have no OS gate. The probe passed on `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-gnu`; only the unselected `process` feature is Linux-specific in the manifest. Targets without 64-bit atomics are outside this approval. | `pass` |
| Maintenance and Rust version | Active/mature and compatible with Rust 1.96 | Exact release MSRV is 1.81; the project toolchain is 1.96. The last release/commit cadence is slower than the alternatives but remains within a mature client, with current dependency maintenance and very high adoption. | `pass` |
| Architectural constraints | Natural Rust design; no network service or async runtime; HTTP behavior safely adaptable | `Registry` + cloneable metric vectors + `Encoder` is the crate's native model and closely matches the observable contract without a Go-API compatibility layer. HTTP transport, header parsing, compression, and status mapping remain in the server crate rather than pulling in a runtime-coupled exporter. | `pass` |

## Candidate comparison

Adoption figures below are official crates.io values captured on 2026-08-11.
They are comparison evidence, not future guarantees.

| Candidate | Hard gates | Adoption and maintenance | Integration/build cost | Decision |
| --- | --- | --- | --- | --- |
| [`prometheus` 0.14.0](https://crates.io/api/v1/crates/prometheus) | Passes all gates, including descriptor consistency and duplicate-registration errors; supplies shared registry plus text and protobuf encoders. | 134.0M total / 26.8M recent downloads; 878 dependent crates; 30.1M downloads for 0.14.0. Mature, lower release cadence, maintained through 2025-10. | One direct client. The explicit `protobuf` feature adds a protobuf runtime but no code generator, network stack, or async runtime. The cross-target probe lock contained 26 third-party packages including the direct crate. | **Selected**: highest direct-client adoption and closest semantics. |
| [`prometheus-client` 0.25.0](https://crates.io/api/v1/crates/prometheus-client) | Fails duplicate-registration behavior. `Registry::register` returns `()` rather than `Result`, and the project's own [compliance list](https://github.com/prometheus/client_rust#specification-compliance) says unique metric-family names are not enforced. Naming/help can be adapted (`_total` and a final help period are added automatically), and text/protobuf support otherwise passes. | 38.5M total / 8.2M recent downloads; [166 dependent crates](https://crates.io/api/v1/crates/prometheus-client/reverse_dependencies?page=1&per_page=1); official Prometheus organization, active release 0.25.0 on 2026-06-15. | Pleasant typed `Family` API and no unsafe in the client, but matching duplicate failure would require a parallel registry/index that duplicates dependency responsibility. | Rejected on a hard behavior gate despite strong maintenance. |
| [`metrics` 0.24.6](https://crates.io/api/v1/crates/metrics) + [`metrics-exporter-prometheus` 0.18.3](https://crates.io/api/v1/crates/metrics-exporter-prometheus) | Fails the registered-vector/duplicate gate. The [`Recorder`](https://docs.rs/metrics/0.24.6/metrics/trait.Recorder.html) returns handles for keys and leaves re-registration/description semantics implementation-defined; repeated metric keys are normal, not registration failures. It can render and its `protobuf` feature negotiates current formats. | `metrics`: 98.5M total and 1,087 dependents. Exporter: 41.1M total / 10.8M recent and 425 dependents. Both had 2026 releases and active maintenance. | Two direct crates. Exporter's default HTTP/push features pull Tokio/Hyper; disabling defaults avoids the runtime but returns to a custom HTTP adapter. Its global-recorder facade is less direct for two fixed registered vectors. | Rejected on hard behavior and architecture gates. |
| [`metrics-prometheus` 0.11.2](https://crates.io/api/v1/crates/metrics-prometheus) | Passes the core behavior gates only when callers use its explicit fallible [`try_register_metric`](https://docs.rs/metrics-prometheus/0.11.2/metrics_prometheus/struct.Recorder.html#method.try_register_metric) path into rust-prometheus; it still provides no HTTP handler. Its documented default registration-failure policy can become a release-mode no-op unless explicitly replaced. | 0.5M total / 0.1M recent downloads; active 2026-07-23 release, MSRV 1.85, MIT OR Apache-2.0. | Adds `metrics`, `metrics-util`, recorder/global-recorder policy, `arc-swap`, and other facade machinery while depending on `prometheus` 0.14 underneath. | Rejected among passing choices: far less adopted and materially more integration surface with no oracle-visible benefit. |
| [`opentelemetry-prometheus` 0.32.0](https://crates.io/api/v1/crates/opentelemetry-prometheus) | Can export through a `prometheus::Registry`, but does not expose the required direct registered vectors or duplicate semantics. The [manifest](https://docs.rs/crate/opentelemetry-prometheus/0.32.0/source/Cargo.toml) depends on `prometheus` 0.14 anyway. | 13.0M total / 2.2M recent downloads and [61 dependent crates](https://crates.io/api/v1/crates/opentelemetry-prometheus/reverse_dependencies?page=1&per_page=1); active 2026 release. | Adds OpenTelemetry API/SDK/instrument-provider machinery and may emit OpenTelemetry metadata for a capability that needs only two Prometheus families. | Rejected: indirect, larger architecture with no behavioral advantage. |
| [`prometheus_exporter` 0.8.5](https://crates.io/api/v1/crates/prometheus_exporter) | Its synchronous embedded server gathers a global/custom rust-prometheus registry, but only uses `TextEncoder`; it does not reproduce promhttp content negotiation, compression, error surface, or self-metric names. It pins older `prometheus` 0.13. | 1.1M total downloads, 15 dependent crates, last release 2022-08-23. | Adds and owns a `tiny_http` server thread, conflicting with the package's existing listener/lifecycle behavior. | Rejected on behavior, maintenance, and architecture gates. |

Framework-specific glue such as `prometheus-hyper` and `axum-prometheus`, derive
wrappers such as `prometheus-metric-storage`, and format parsers are not separate
credible clients for this request: they either wrap one of the candidates above,
couple to an async web framework, or do not own collection/registration. None
fixes a hard-gate failure with lower cost than the selected direct client.

## Selected integration

### Required features and configuration

Use exactly:

```toml
prometheus = { version = "=0.14.0", default-features = false, features = ["protobuf"] }
```

Do not enable `gen`, `nightly`, `process`, or `push`. The integrator, not a
package implementor or dependency researcher, owns the workspace manifest and
lockfile change.

### Natural API model

- Let `internal/metrics` own one process-wide `Registry` and the registered
  handles. A single fallible initializer (typically held by `OnceLock`) should
  build and register the metrics once; `internal/machine/metrics` gathers that
  same registry. Using `prometheus::default_registry()` is also approved when
  the workspace deliberately chooses the crate-global registry.
- Build `GaugeVec` from
  `Opts::new("build_info", "Build information.").namespace("uncloud").subsystem("uncloudd")`
  with `&["version"]`, then set the selected version child to `1.0`.
- Build `IntCounterVec` from
  `Opts::new("query_total", "Counter of DNS queries.").namespace("uncloud").subsystem("dns")`
  with `&["internal", "status"]`; use label values exactly `"true"|"false"`
  and `"ok"|"err"`.
- Registration errors, including `Error::AlreadyReg` and inconsistent help or
  labels for an existing fully qualified name, must fail initialization. Do not
  silently replace or merge application collectors.

The resulting required text families are:

```text
# HELP uncloud_dns_query_total Counter of DNS queries.
# TYPE uncloud_dns_query_total counter
uncloud_dns_query_total{internal="false",status="ok"} 1
# HELP uncloud_uncloudd_build_info Build information.
# TYPE uncloud_uncloudd_build_info gauge
uncloud_uncloudd_build_info{version="<version>"} 1
```

### HTTP exposition obligations for `internal/machine/metrics`

`prometheus` intentionally provides encoders rather than an HTTP handler. The
server crate must keep a thin adapter around its approved synchronous HTTP
server:

1. Gather the same shared registry on `/metrics` for every scrape.
2. Parse `Accept` quality values and exact media-type parameters as Go's
   negotiator does. Accept `application/vnd.google.protobuf` only with
   `proto=io.prometheus.client.MetricFamily` and `encoding=delimited`,
   `encoding=text`, or `encoding=compact-text`; otherwise fall back to
   Prometheus text `0.0.4`. Honor the recognized `escaping` parameter
   (`allow-utf-8`, `underscores`, `dots`, or `values`) and otherwise use the
   Go default `underscores`; append the selected scheme to the response
   content type. The requested metric and label names are already legacy-valid,
   so all four schemes produce the same payload names. Use `ProtobufEncoder`
   for delimited form. Under the selected `protobuf` feature, the public
   gathered `MetricFamily` implements
   normal and alternate `Display`, which provides compact and pretty protobuf
   text for the two legacy forms; lock their byte-level behavior in tests.
   OpenMetrics is not enabled by the oracle's zero-value `HandlerOpts`.
3. Use the selected protobuf formatting or `TextEncoder` and set the exact
   negotiated `Content-Type`. For text, set
   `text/plain; version=0.0.4; charset=utf-8; escaping=<scheme>`; the crate
   constant omits the `charset` and `escaping` parameters even though the
   encoded bytes are correct for these legacy-valid names.
4. Preserve Go's zero-value `HTTPErrorOnError` behavior. A collection error
   before response commitment returns HTTP 500, removes `Content-Encoding`, and
   uses `Content-Type: text/plain; charset=utf-8` with body
   `An error has occurred while serving metrics:\n\n<error>\n`; an encoding or
   write error after the response begins aborts the response rather than trying
   to replace it with a 500. The selected registry's built-in collectors cannot
   fail during gather (`gather` returns a `Vec`, not `Result`), while encoder
   errors remain explicit.
5. Preserve Go Handler's observable scrape self-metrics:
   `promhttp_metric_handler_requests_total{code}` (initialize `200`, `500`, and
   `503`) and `promhttp_metric_handler_requests_in_flight`. Construct the handler
   once, because rust-prometheus reports `AlreadyReg` but does not return the
   already-registered collector for Go's idempotent reuse pattern.
6. The metrics crate does not implement `Accept-Encoding`. The HTTP layer must
   reproduce the oracle's quality-aware `identity`, gzip, and zstd negotiation,
   including identity fallback and setting `Content-Encoding` only for a
   compressed response. If the package's approved HTTP/compression facilities
   cannot do so, return that transport need to the dependency gate; do not
   silently omit it, add an async runtime, or create a second metrics registry.

### Known limitations

- The crate has no ready-made `promhttp.Handler` equivalent. Accept parsing,
  exact headers, compression, HTTP error mapping, and handler self-metrics are
  explicit server-package responsibilities and must receive HTTP-level parity
  tests.
- There is no dedicated encoder type for Go expfmt's legacy protobuf
  `encoding=text` and `encoding=compact-text`; the adapter must use the gathered
  protobuf model's pretty/compact formatting and prove wire parity in tests.
  OpenMetrics remains intentionally unsupported because the oracle's handler
  does not enable it.
- `Registry::gather()` cannot express a collector gathering error because the
  Rust `Collector::collect` contract is infallible. This is equivalent for the
  selected built-in gauge/counter collectors, but not for a future fallible
  custom collector.
- Metric-vector children remain allocated after first use unless explicitly
  removed. Keep the requested label domains bounded (`version`, boolean
  `internal`, and two status values).
- Exact 0.14.0 is mature but lower-cadence than `prometheus-client` and the
  `metrics` ecosystem. Re-review on an upgrade rather than floating the version.

## Verification command or probe

An isolated crate was built with Rust/Cargo 1.96 and the exact dependency line
above. Its probe:

- registered the two vectors on a fresh `Registry`;
- asserted a second registration returned `Error::AlreadyReg`;
- spawned eight threads, each performing 10,000 increments of the same labeled
  counter and concurrent `set(1.0)` calls on the same labeled gauge;
- asserted the complete text output shown above, with counter value `80000`;
- asserted non-empty delimited protobuf output and both encoder content-type
  constants;
- registered a uniquely named vector with `default_registry()` and observed it
  through global `gather()`.

Commands run successfully:

```sh
rustc --version
# rustc 1.96.0 (ac68faa20 2026-05-25)
cargo run --locked --manifest-path /tmp/ployz-prometheus-probe.maKg2c/Cargo.toml
cargo check --locked --manifest-path /tmp/ployz-prometheus-probe.maKg2c/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check --locked --manifest-path /tmp/ployz-prometheus-probe.maKg2c/Cargo.toml --target x86_64-pc-windows-gnu
```

Package acceptance must add permanent crate-local tests for the same assertions,
plus HTTP tests covering quality-weighted text and all three protobuf encodings,
exact content types, gzip/zstd/identity negotiation, 500 error body, and handler
self-metrics.

## Review

A second adversarial dependency reviewer is **not required**. The workflow's
critical list does not include this capability, and the selected dependency has
no network service, async runtime, cryptography, storage, container control, or
enabled unsafe FFI. The HTTP listener remains owned by the separately reviewed
server package; this decision approves the metrics model and encoders plus the
bounded adapter behavior above.

Affected packages (package packets do not yet exist at the researched base):

- `internal/metrics`
- `internal/machine/metrics`
