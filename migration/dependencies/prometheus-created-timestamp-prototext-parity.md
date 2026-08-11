# Dependency decision: `prometheus-created-timestamp-prototext-parity`

| Field | Value |
| --- | --- |
| Status | `human-decision-required` |
| Capability | Prometheus counter creation timestamps and legacy protobuf text/compact-text parity |
| Selected dependency | None passes every hard gate. Closest safe wire/canonical-text seam: retain `prometheus = { version = "=0.14.0", default-features = false, features = ["protobuf"] }` and declare its resolved runtime directly as `protobuf = { version = "=3.7.2", default-features = false }`; this still requires an `internal/metrics` change, a narrow local formatter, and an explicit text-whitespace deviation. |
| License | `Apache-2.0 AND MIT` for the closest seam (`prometheus` and `protobuf`, respectively) |
| Research date | `2026-08-11` UTC |
| Request | [`migration/dependencies/requests/prometheus-created-timestamp-prototext-parity.md`](requests/prometheus-created-timestamp-prototext-parity.md) |

## Verdict

No current Rust metrics client or protobuf text adapter reproduces the complete
frozen Go behavior inside `internal/machine/metrics` alone.

Exact creation-timestamp semantics and delimited-wire parity for the
application and handler families are technically possible with safe public
APIs, but they are not a dependency-only or package-local change:

1. `internal/metrics` must record `SystemTime` when each counter-vector child is
   first created and place the corresponding encoded `Timestamp` in public
   rust-protobuf unknown field 3 during collection.
2. `internal/machine/metrics` must use a deliberately narrow, deterministic
   formatter for the two legacy protobuf text variants. This requires a
   human-approved whitespace contract: protobuf-go 1.36.9 deliberately adds an
   extra space according to a hash of the program binary so text bytes remain
   stable within one binary but change across builds. A deterministic Rust
   formatter cannot also match every valid frozen-dependency Go build byte for
   byte without imitating private Go runtime state.

The first item crosses an already-integrated package boundary. The current
`ployz-internal-metrics` API returns ordinary `prometheus::IntCounter` children
and exposes a `Registry`; neither contains nor exposes the discarded creation
instant. Assigning a timestamp at first scrape would be observably later than
Go's first counter-child access and is rejected as fabricated parity.

This intentional upstream instability means there is no build-independent
"exact Go text bytes" target for a dependency to pass. The safe resolution is
to preserve the protobuf meaning and adopt a fixed canonical spacing in Rust;
that is a visible deviation and is not inferred here.

There is also a pre-existing endpoint-level difference that the narrow request
did not enumerate. Go's default registry automatically gathers 38 `go_*` and
`process_*` families in the probe environment, including one summary. The
selected Rust client, with `process` disabled, does not create these families;
and Go runtime values have no honest Rust equivalent. None of those default Go
families carried a created timestamp in the probe, so they do not change the
field-3 result, but their absence prevents either option below from claiming
complete endpoint byte equality. Strict full-endpoint parity therefore needs a
separate runtime/process-metrics decision; this record does not invent those
metrics or infer approval to omit them.

Therefore no dependency is approved and no deviation is inferred. A human must
choose one of the precise resolutions under [Required human decision](#required-human-decision).

## Required behavior and source evidence

- The oracle installs an unmodified `promhttp.Handler()` at `/metrics` in
  [`server.go`](../../upstream/uncloud/internal/machine/metrics/server.go). The
  frozen [`expfmt` v0.62.0 encoder](https://github.com/prometheus/common/blob/v0.62.0/expfmt/encode.go#L149-L185)
  uses delimited protobuf, generated-message `String()` for `compact-text`, and
  Go `prototext.Format` for `text`.
- The frozen [`client_model` v0.6.1 schema](https://github.com/prometheus/client_model/blob/v0.6.1/io/prometheus/client/metrics.proto#L42-L51)
  declares `Counter.created_timestamp` as message field 3. The frozen
  [`client_golang` v1.22 counter implementation](https://github.com/prometheus/client_golang/blob/v1.22.0/prometheus/counter.go#L90-L120)
  records `time.Now()` in each scalar counter, and
  [`NewCounterVec`](https://github.com/prometheus/client_golang/blob/v1.22.0/prometheus/counter.go#L200-L230)
  records it when a label-value child is first created. Repeated gathers return
  that same instant.
- Both legacy text paths ultimately use protobuf-go's internal text encoder.
  Its [`prepareNext`](https://github.com/protocolbuffers/protobuf-go/blob/v1.36.9/internal/encoding/text/encode.go#L225-L244)
  deliberately adds a pseudo-random extra space to make output unstable. The
  associated [`detrand`](https://github.com/protocolbuffers/protobuf-go/blob/v1.36.9/internal/detrand/rand.go)
  is seeded from an approximate hash of the executable: output is stable within
  one program and intentionally unstable across different builds. Rebuilding
  the same fixture after changing its imports changed both text hashes while
  leaving raw and delimited protobuf hashes unchanged.
- The only application counter is `DNSQuery`, declared in
  [`internal/metrics/metrics.go`](../../upstream/uncloud/internal/metrics/metrics.go)
  and first accessed by the forwarded and internal DNS paths in
  [`dns/server.go`](../../upstream/uncloud/internal/machine/dns/server.go).
  The build-info metric is a gauge and has no created timestamp. The promhttp
  request counters are created while constructing the handler: the frozen
  [`InstrumentMetricHandler`](https://github.com/prometheus/client_golang/blob/v1.22.0/prometheus/promhttp/http.go#L286-L304)
  initializes children `200`, `500`, and `503`. The probe confirmed all three
  carry created timestamps, and the Rust server package can track their exact
  local creation instants.
- Go's default registry also auto-registers Go and process collectors. The
  executable inventory contained 38 families: gauges and counters plus
  `go_gc_duration_seconds` as a summary. None of their counter, summary, or
  histogram DTOs had `created_timestamp` set in the frozen-version probe. They
  remain reachable through the oracle's unmodified default handler and are an
  explicit endpoint-level divergence, not formatter input the Rust package may
  silently reject.
- The direct machine caller constructs the metrics server in
  [`machine.go`](../../upstream/uncloud/internal/machine/machine.go) and the
  cluster controller runs it in
  [`cluster.go`](../../upstream/uncloud/internal/machine/cluster.go). Callers do
  not supply a registry snapshot or creation metadata.
- The integrated Rust metrics crate stores only `GaugeVec` and `IntCounterVec`
  and returns ordinary child handles in
  [`crates/ployz-internal-metrics/src/lib.rs`](../../crates/ployz-internal-metrics/src/lib.rs).
  Its public `registry()` preserves collection but cannot recover the instant
  discarded at child creation.

## Primary-source evidence

- [`prometheus` 0.14.0's bundled schema](https://github.com/tikv/rust-prometheus/blob/v0.14.0/proto/proto_model.proto)
  predates exemplars and created timestamps: `Counter` contains only field 1,
  `value`. Its exact
  [manifest](https://docs.rs/crate/prometheus/0.14.0/source/Cargo.toml.orig)
  declares Apache-2.0, MSRV 1.81, and resolves rust-protobuf through the selected
  `protobuf` feature. Official crates.io data captured on 2026-08-11 reported
  134,154,048 total downloads, 26,980,777 recent downloads, and 879 dependent
  crates. Version 0.14.0 remains the latest stable release.
- rust-protobuf 3.7.2 exposes safe public
  [`UnknownFields::add_length_delimited`](https://docs.rs/protobuf/3.7.2/protobuf/struct.UnknownFields.html#method.add_length_delimited)
  and [`UnknownFields::get`](https://docs.rs/protobuf/3.7.2/protobuf/struct.UnknownFields.html#method.get).
  The probe injected an encoded `google.protobuf.Timestamp` as counter field 3
  and produced both the exact raw message and the exact length-delimited Go
  frame through `ProtobufEncoder`, without accessing private state or writing
  application `unsafe`. A direct dependency adds no runtime packages because
  `prometheus[protobuf]` already resolves `protobuf` 3.7.2.
- rust-protobuf's public
  [`text_format`](https://docs.rs/protobuf/3.7.2/protobuf/text_format/index.html)
  formats its generated or dynamic messages, but its compact and pretty bytes
  are not Go's bytes. In the golden fixture it prints `metric { ... }` instead
  of Go's `metric:{...}` / `metric: { ... }`, uses different colon spacing,
  and does not add Go's extra pretty-format newline. Its old generated
  descriptor also renders the timestamp as numeric unknown field `3` unless a
  dynamic descriptor is constructed.
- [`prost-reflect` 0.16.5](https://docs.rs/prost-reflect/0.16.5/prost_reflect/)
  is actively maintained, MSRV 1.82, MIT OR Apache-2.0, and denies unsafe code.
  Its public
  [`FormatOptions`](https://docs.rs/prost-reflect/0.16.5/prost_reflect/text_format/struct.FormatOptions.html)
  supports compact and pretty protobuf text. However, its documented canonical
  syntax and the probe use comma/list notation, omit message-field colons, and
  render integral doubles as `7.0`; all differ from Go. Official crates.io data
  captured 71,987,021 total downloads, 16,388,985 recent downloads, and 148
  dependent crates.
- [`prometheus-client` 0.25.0](https://github.com/prometheus/client_rust/tree/v0.25.0)
  is the actively maintained official Prometheus Rust client, but its
  [`prometheus_protobuf` encoder](https://github.com/prometheus/client_rust/blob/v0.25.0/src/encoding/prometheus_protobuf.rs#L239-L267)
  sets the ordinary counter timestamp to `None`. Its bundled schema names wire
  field 3 `start_timestamp`, so even a populated field would have the wrong
  legacy text name. It also already failed the selected registry's required
  duplicate-registration semantics in the approved
  [`prometheus-metrics`](prometheus-metrics.md) decision. Official crates.io
  data captured 38,601,324 total downloads, 8,297,376 recent downloads, and 166
  dependent crates.
- Google `protobuf`/`protobuf-codegen` `4.35.1-release` can generate the current
  schema, but its Rust runtime builds and links C upb and routes debug text
  through unsafe FFI. The primary-source and executable evidence is already
  recorded in [`protobuf-codegen-runtime`](protobuf-codegen-runtime.md). It
  also supplies debug text, not both exact Go legacy encoder modes, and would
  introduce a second incompatible protobuf runtime beside rust-prometheus.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Preserve each counter child's creation instant and exact delimited, Go protobuf text, and Go compact-text bytes | Go captures the instant on first child access. The waiting server sees only gathered DTOs after the integrated Rust client discarded that instant. Public field-3 injection passes wire parity when metadata is supplied. Every surveyed text formatter differs, and protobuf-go intentionally varies whitespace between builds, so no single deterministic byte target exists. The oracle also exposes 38 default Go/process families absent from the Rust registry. | `fail`; exact timestamps and wire are feasible, exact build-independent legacy text bytes are not |
| License and security | Permissive licenses; no unsound/private access; acceptable advisory state | Closest seam is Apache-2.0 plus MIT and uses safe public APIs. `cargo audit` found no vulnerabilities in the exact 42-package probe lock. `protobuf` 3.7.2 contains internal unsafe Rust but the selected path requires no application unsafe or native FFI. Google v4 is rejected for C/unsafe FFI. | `pass` for closest seam |
| Platforms and targets | Rust 1.96 and existing supported targets | The exact probe passed `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-gnu`; the selected APIs have no OS gate. `SystemTime`, a lock around bounded label metadata, and byte formatting are portable. | `pass` |
| Maintenance and Rust version | Maintained or mature dependencies compatible with Rust 1.96 | `prometheus` 0.14.0 has MSRV 1.81 and very high adoption but a slow release cadence. `protobuf` 3.7.2 is the already-resolved runtime. `prost-reflect` and `prometheus-client` are current but fail behavior. | `pass` for closest seam; re-review on upgrade |
| Deterministic encoding | Stable bytes for identical DTOs and timestamps | The fixed Go raw message round-tripped byte-for-byte through rust-protobuf, and `ProtobufEncoder` added the same uvarint length prefix as Go's delimited encoder. Unknown field 3 is the sole unknown field on the bounded counter DTO, and known/repeated field order is stable. Protobuf-go intentionally destabilizes text whitespace across binaries; exact Go text parity and a build-independent deterministic contract are mutually incompatible. | `pass` for raw/delimited wire with supplied metadata; `human-decision-required` for canonical text |
| Architectural constraints | Natural Rust design, bounded seam, no second registry/runtime, and implementable by the assigned owner | A collector wrapper plus direct use of the already-resolved runtime is bounded and safe, but recording the correct DNS child instant belongs in the already-integrated `internal/metrics` crate. The waiting package cannot manufacture it. Recreating Go runtime collectors in Rust would be misleading and is outside this capability. | `fail` under current assignment |

## Candidate comparison

| Candidate | Behavior result | Integration and maintenance | Decision |
| --- | --- | --- | --- |
| Existing `prometheus = 0.14.0` plus direct `protobuf = 3.7.2` and public unknown-field injection | Exact delimited bytes when a correct timestamp is supplied. Built-in static/dynamic text formatters differ from both Go text modes, for which upstream itself has no cross-build-stable whitespace. No creation metadata is available to the waiting package. | Smallest graph and highest client adoption; direct `protobuf` adds no resolved package. Requires a narrow canonical formatter and an `internal/metrics` collector wrapper. | **Closest seam; needs human authorization for both the cross-package change and canonical-text deviation.** |
| `prometheus-client = 0.25.0`, feature `prometheus_protobuf` | Ordinary counters encode `start_timestamp: None`; field name is `start_timestamp`, not Go's `created_timestamp`; registry semantics already fail. | Active official client but requires replacing the integrated registry/client and adds Prost build/codegen. | Rejected on behavior and architecture hard gates. |
| `prost-reflect = 0.16.5`, feature `text-format`, with `prost = 0.14.4` | Understands the current DTO descriptor and timestamp, but both compact and pretty bytes fail the golden fixture. It cannot restore a discarded creation instant. | Pure Rust and current, but adds a second protobuf runtime plus lexer/reflection machinery for output a bounded writer can cover. | Rejected on behavior and build-cost grounds. |
| rust-protobuf 3.7.2 dynamic descriptor plus its text formatter | Preserves and names field 3, but exact syntax and newline bytes differ in both modes. | Reuses the resolved runtime and public reflection, but dynamic descriptor construction adds complexity without passing behavior. | Rejected as formatter; retain only as the closest wire seam. |
| Google `protobuf`/codegen `4.35.1-release` | Current typed field is available, but the runtime does not supply both exact Go encoder modes. | Native C upb, unsafe FFI, exact protoc coupling, second protobuf runtime. | Rejected on behavior, security/portability, and architecture gates. |
| Package-local first-scrape timestamp cache | Field presence and stability can be simulated after first observation. The value is later than Go's actual counter creation and changes behavior based on scrape timing. | Simple but semantically false and race-sensitive. | Rejected; not offered as a deviation. |

No published `prometheus-client-model` Rust crate exists in the crates.io index
at research time. Parsers, framework exporters, OpenTelemetry bridges, and
generic metrics facades do not own both the missing creation metadata and a
bounded canonical legacy encoder, so they are not separate credible candidates.

## Required human decision

### Option A — exact timestamps/wire plus canonical legacy text (recommended)

Authorize the controller to return `internal/metrics` to fixing and coordinate
these exact dependencies:

```toml
prometheus = { version = "=0.14.0", default-features = false, features = ["protobuf"] }
protobuf = { version = "=3.7.2", default-features = false }
```

- Wrap each counter-vector child creation in a bounded, synchronized metadata
  map keyed by its exact label values. Record `SystemTime::now()` only for the
  winning first insertion, mirroring Go's first child creation and reuse.
- During collection, encode that time as `google.protobuf.Timestamp` and add it
  as counter unknown field 3 through public rust-protobuf APIs. Deletion/reset
  must replace the timestamp if those operations are ever exposed; current
  callers neither delete nor reset.
- Apply the same wrapper locally to the promhttp request counters at handler
  construction.
- In `internal/machine/metrics`, emit delimited bytes from the augmented DTO and
  implement a deterministic bounded gauge/counter legacy text writer required
  by the shared Rust registry. If a summary, histogram, untyped metric, exemplar,
  unexpected unknown field, invalid timestamp, or unsupported float case is
  ever present in that registry, it must return an encoder error until the
  corresponding Go semantic fixture and formatter branch are added. The known
  Go default summary is not registered in Rust and is covered by the explicit
  family-set deviation below, so the endpoint must not partially fabricate it.
- Specify and lock one canonical spacing policy for empty/non-empty labels,
  escaped strings, zero and positive values, field ordering, one newline for
  compact text, and two newlines for pretty text. Parse both the Go and Rust
  outputs against the current descriptor and require equivalent messages;
  compare bytes only after normalizing protobuf-go's optional extra spaces.

Exact deviations that still require human acceptance under Option A:

- the Rust endpoint omits all 29 `go_*` and nine `process_*` families observed
  in the frozen Go default registry; no Go runtime value is fabricated;
- legacy text spacing follows the selected stable Rust canonical form rather
  than protobuf-go's executable-hash-dependent extra spaces;
- only families actually registered in Rust are wire-compatible. The
  application and promhttp counter timestamps are exact by construction, but
  whole-response bytes cannot match a response with the omitted Go-specific
  families.

The benefit is no timestamp or delimited-wire deviation for the requested
shared families. The cost is expanded package ownership, a maintained
output-format seam, and the explicit whitespace deviation that makes text
encoding deterministic across Rust builds.

### Option B — keep package ownership, accept additional timestamp omission

Keep `internal/metrics` integrated and authorize the waiting package to omit
`Counter.created_timestamp` only from `ployz_dns_query_total` children, whose
true creation instants were discarded by that crate. The server owns creation
of `promhttp_metric_handler_requests_total` and must still capture and encode
field 3 correctly for its `200`, `500`, and `503` children. Still require a
narrow canonical formatter with the same explicit whitespace deviation as
Option A; do not accept rust-protobuf or prost-reflect syntax differences as an
accidental additional deviation.

Exact deviation: each DNS-query counter DTO lacks field 3, so both legacy text
forms omit `created_timestamp:{...}` / `created_timestamp: {...}` for those
samples and the raw protobuf message lacks tag 3. The delimited framing protocol
is unchanged, but the affected `Counter`, enclosing `Metric`, enclosing
`MetricFamily`, and outer frame length values decrease, changing their encoded
bytes solely because that field is absent; a uvarint prefix need not occupy
fewer octets. Counter values, labels, family ordering among registered Rust
families, content types, and unaffected family bytes stay in contract.

Option B also retains Option A's canonical-text spacing and omission of the 38
Go-specific default families. It therefore has three independently visible
deviations: text whitespace, runtime-family omission, and missing DNS counter
creation timestamps.

If the human instead requires strict full-response equality including all
default Go/process families, select neither option: leave this capability
blocked and open a separate runtime/process-collector request before resuming
the waiting package.

If the human requires byte-for-byte legacy text equality across arbitrary Go
rebuilds, select neither option: protobuf-go deliberately provides no such
stable output. Pinning one particular Go executable's whitespace would create a
binary-specific golden, not Go-format parity, and is not recommended.

### Not authorized

- A first-scrape or server-start timestamp presented as the counter creation
  instant.
- A patched/forked metrics client, private-field access, application `unsafe`,
  Google v4 native FFI, or a second registry.
- Treating the controller's general provisional policy as approval for either
  option; this record intentionally awaits an explicit human choice.

## Golden probe and verification

The Go probe used Go 1.26.1 with the frozen versions (`client_golang` 1.22.0,
`client_model` 0.6.1, `common` 0.62.0, and protobuf 1.36.9). It built one fixed
counter family with label value `a\\n\\\"b`, value `7`, and timestamp
`1700000000s + 123456789ns`, then encoded all three protobuf variants. Raw and
delimited payloads are stable golden fixtures. The compact/text values below
are observations from the exact embedded probe build, not portable goldens:

```text
raw message sha256 3cedfe0a7600b108b294ffbfdc854b6a7794e93e7e48023f3214c134be0dd41a
delimited sha256   0dce39c99ecb4c9f90f19302632ea029bbffaeb20fea170df429f328b162055d
observed compact sha256 893490ff6f6fb2f975c7a1a5e87431a29b678690c2c8020e2a47bca697ebc9d9
observed text sha256    4d84bf1e1cabb30d41e5df492e89d9246299078a1f109764ef9aa15cfa2cf1a2
raw message base64 Cgtwcm9iZV90b3RhbBIGUHJvYmUuGAAiKAoOCgRraW5kEgZhXG5cImIaFgkAAAAAAAAcQBoLCIDiz6oGEJWa7zo=
delimited base64   QQoLcHJvYmVfdG90YWwSBlByb2JlLhgAIigKDgoEa2luZBIGYVxuXCJiGhYJAAAAAAAAHEAaCwiA4s+qBhCVmu86
observed compact base64 bmFtZToicHJvYmVfdG90YWwiICBoZWxwOiJQcm9iZS4iICB0eXBlOkNPVU5URVIgIG1ldHJpYzp7bGFiZWw6e25hbWU6ImtpbmQiICB2YWx1ZToiYVxcblxcXCJiIn0gIGNvdW50ZXI6e3ZhbHVlOjcgIGNyZWF0ZWRfdGltZXN0YW1wOntzZWNvbmRzOjE3MDAwMDAwMDAgIG5hbm9zOjEyMzQ1Njc4OX19fQo=
observed text base64    bmFtZTogICJwcm9iZV90b3RhbCIKaGVscDogICJQcm9iZS4iCnR5cGU6ICBDT1VOVEVSCm1ldHJpYzogIHsKICBsYWJlbDogIHsKICAgIG5hbWU6ICAia2luZCIKICAgIHZhbHVlOiAgImFcXG5cXFwiYiIKICB9CiAgY291bnRlcjogIHsKICAgIHZhbHVlOiAgNwogICAgY3JlYXRlZF90aW1lc3RhbXA6ICB7CiAgICAgIHNlY29uZHM6ICAxNzAwMDAwMDAwCiAgICAgIG5hbm9zOiAgMTIzNDU2Nzg5CiAgICB9CiAgfQp9Cgo=
```

Repeated runs of that unchanged executable produced the same observed text
hashes. Changing the probe binary changed the presence of optional extra spaces
and both text hashes, exactly as protobuf-go's `detrand` source specifies; raw
and delimited hashes remained unchanged. Acceptance tests must therefore treat
the text payloads as semantic/normalization fixtures unless a human selects a
specific canonical Rust spacing policy.

The runtime portion asserted that the Go-created timestamp was non-null,
between the calls bracketing first child access, and unchanged across two
gathers. It also constructed a handler on a fresh registry and reported
`PROMHTTP_REQUEST_CHILDREN=3 ALL_CREATED=true`. The Rust 1.96 probe then
established:

```text
PUBLIC_UNKNOWN_INJECTION_MATCH=true
DELIMITED_B64=<the exact delimited base64 above>
DYNAMIC_ROUNDTRIP=<the exact raw-message base64 above>
rust-protobuf compact starts: name: "probe_total" ... metric { ... }
prost-reflect compact starts: name:"probe_total",help:"Probe.",...
```

Commands completed successfully:

```sh
GOTOOLCHAIN=local /opt/go1.26.1/bin/go run .
# from /tmp/ployz-prometheus-created-probe/go

cargo run --locked --manifest-path /tmp/ployz-prometheus-created-probe/rust/Cargo.toml
cargo fmt --manifest-path /tmp/ployz-prometheus-created-probe/rust/Cargo.toml --check
cargo check --locked --manifest-path /tmp/ployz-prometheus-created-probe/rust/Cargo.toml --all-targets
cargo clippy --locked --manifest-path /tmp/ployz-prometheus-created-probe/rust/Cargo.toml --all-targets -- -D warnings
cargo check --locked --manifest-path /tmp/ployz-prometheus-created-probe/rust/Cargo.toml --target x86_64-pc-windows-gnu
cargo audit --file /tmp/ployz-prometheus-created-probe/rust/Cargo.lock

cargo test -p ployz-internal-metrics --all-targets
cargo clippy -p ployz-internal-metrics --all-targets --all-features -- -D warnings
```

The `/tmp` paths are only scratch locations. The durable reconstruction inputs
are the exact versions above, the raw/delimited base64 fixtures, the observed
build-specific text values, and these minimal sources.
For the Go module, use the four frozen requirements listed above and this
program body:

```go
package main

import (
	"bytes"
	"encoding/base64"
	"fmt"
	"time"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promhttp"
	dto "github.com/prometheus/client_model/go"
	"github.com/prometheus/common/expfmt"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/timestamppb"
)

func main() {
	family := &dto.MetricFamily{
		Name: proto.String("probe_total"), Help: proto.String("Probe."),
		Type: dto.MetricType_COUNTER.Enum(), Metric: []*dto.Metric{{
			Label: []*dto.LabelPair{{Name: proto.String("kind"), Value: proto.String("a\\n\\\"b")}},
			Counter: &dto.Counter{Value: proto.Float64(7), CreatedTimestamp: &timestamppb.Timestamp{
				Seconds: 1700000000, Nanos: 123456789,
			}},
		}},
	}
	wire, _ := proto.Marshal(family)
	fmt.Printf("WIRE_B64=%s\n", base64.StdEncoding.EncodeToString(wire))
	var delimited bytes.Buffer
	if err := expfmt.NewEncoder(&delimited, expfmt.FmtProtoDelim).Encode(family); err != nil { panic(err) }
	fmt.Printf("DELIMITED_B64=%s\n", base64.StdEncoding.EncodeToString(delimited.Bytes()))
	for _, item := range []struct{name string; format expfmt.Format}{
		{"COMPACT", expfmt.FmtProtoCompact}, {"TEXT", expfmt.FmtProtoText},
	} {
		var out bytes.Buffer
		if err := expfmt.NewEncoder(&out, item.format).Encode(family); err != nil { panic(err) }
		fmt.Printf("%s_B64=%s\n", item.name, base64.StdEncoding.EncodeToString(out.Bytes()))
	}

	before := time.Now()
	registry := prometheus.NewRegistry()
	vec := prometheus.NewCounterVec(prometheus.CounterOpts{Name: "runtime_total", Help: "Runtime."}, []string{"kind"})
	registry.MustRegister(vec)
	vec.WithLabelValues("x").Inc()
	after := time.Now()
	one, _ := registry.Gather()
	time.Sleep(time.Millisecond)
	two, _ := registry.Gather()
	created1 := one[0].Metric[0].Counter.CreatedTimestamp.AsTime()
	created2 := two[0].Metric[0].Counter.CreatedTimestamp.AsTime()
	fmt.Printf("RUNTIME_CREATED_PRESENT=%t RANGE=%t STABLE=%t\n",
		one[0].Metric[0].Counter.CreatedTimestamp != nil,
		!created1.Before(before) && !created1.After(after), created1.Equal(created2))
	handlerRegistry := prometheus.NewRegistry()
	promhttp.InstrumentMetricHandler(handlerRegistry,
		promhttp.HandlerFor(handlerRegistry, promhttp.HandlerOpts{}))
	handlerFamilies, _ := handlerRegistry.Gather()
	for _, handlerFamily := range handlerFamilies {
		if handlerFamily.GetName() == "promhttp_metric_handler_requests_total" {
			allCreated := true
			for _, metric := range handlerFamily.Metric {
				allCreated = allCreated && metric.Counter.CreatedTimestamp != nil
			}
			fmt.Printf("PROMHTTP_REQUEST_CHILDREN=%d ALL_CREATED=%t\n",
				len(handlerFamily.Metric), allCreated)
		}
	}

	defaults, _ := prometheus.DefaultGatherer.Gather()
	for _, defaultFamily := range defaults {
		created := false
		for _, metric := range defaultFamily.Metric {
			created = created || (metric.Counter != nil && metric.Counter.CreatedTimestamp != nil) ||
				(metric.Summary != nil && metric.Summary.CreatedTimestamp != nil) ||
				(metric.Histogram != nil && metric.Histogram.CreatedTimestamp != nil)
		}
		fmt.Printf("DEFAULT_FAMILY=%s TYPE=%s CREATED=%t\n",
			defaultFamily.GetName(), defaultFamily.GetType(), created)
	}
}
```

The Rust comparison manifest and essential source are:

```toml
[package]
name = "ployz-prometheus-created-probe"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false

[dependencies]
base64 = "=0.22.1"
prometheus = { version = "=0.14.0", default-features = false, features = ["protobuf"] }
protobuf = { version = "=3.7.2", default-features = false }
prost = "=0.14.4"
prost-reflect = { version = "=0.16.5", default-features = false, features = ["text-format"] }
```

```rust
use base64::{Engine as _, engine::general_purpose::STANDARD};
use prometheus::{Encoder, ProtobufEncoder, proto::MetricFamily};
use prost_reflect::{DescriptorPool, DynamicMessage, text_format::FormatOptions};
use protobuf::Message;
use protobuf::descriptor::FieldDescriptorProto;
use protobuf::descriptor::field_descriptor_proto::{Label, Type};
use protobuf::reflect::FileDescriptor;
use protobuf::text_format::{print_to_string, print_to_string_pretty};
use protobuf::well_known_types::timestamp;

fn main() {
    let wire = STANDARD.decode(
        "Cgtwcm9iZV90b3RhbBIGUHJvYmUuGAAiKAoOCgRraW5kEgZhXG5cImIaFgkAAAAAAAAcQBoLCIDiz6oGEJWa7zo="
    ).unwrap();
    let old = MetricFamily::parse_from_bytes(&wire).unwrap();

    let mut injected = old.clone();
    let counter = injected.metric[0].counter.as_mut().unwrap();
    counter.mut_unknown_fields().clear();
    let mut created = timestamp::Timestamp::new();
    created.seconds = 1_700_000_000;
    created.nanos = 123_456_789;
    counter.mut_unknown_fields().add_length_delimited(3, created.write_to_bytes().unwrap());
    println!("PUBLIC_UNKNOWN_INJECTION_MATCH={}", injected.write_to_bytes().unwrap() == wire);
    let mut delimited = Vec::new();
    ProtobufEncoder::new().encode(&[injected.clone()], &mut delimited).unwrap();
    println!("DELIMITED_B64={}", STANDARD.encode(delimited));

    let generated = <MetricFamily as protobuf::MessageFull>::descriptor();
    let mut file = generated.file_descriptor_proto().clone();
    file.dependency.push("google/protobuf/timestamp.proto".to_owned());
    let counter = file.message_type.iter_mut().find(|m| m.name() == "Counter").unwrap();
    let mut field = FieldDescriptorProto::new();
    field.set_name("created_timestamp".to_owned());
    field.set_json_name("createdTimestamp".to_owned());
    field.set_number(3);
    field.set_label(Label::LABEL_OPTIONAL);
    field.set_type(Type::TYPE_MESSAGE);
    field.set_type_name(".google.protobuf.Timestamp".to_owned());
    counter.field.push(field);
    let dynamic_file = FileDescriptor::new_dynamic(
        file, &[timestamp::file_descriptor().clone()]
    ).unwrap();
    let dynamic = dynamic_file.message_by_package_relative_name("MetricFamily")
        .unwrap().parse_from_bytes(&wire).unwrap();
    println!("DYNAMIC_COMPACT={}", print_to_string(&*dynamic));
    println!("DYNAMIC_PRETTY={}", print_to_string_pretty(&*dynamic));

    let pool = DescriptorPool::decode(include_bytes!("../metrics.pb").as_ref()).unwrap();
    let desc = pool.get_message_by_name("io.prometheus.client.MetricFamily").unwrap();
    let prost_dynamic = DynamicMessage::decode(desc, wire.as_slice()).unwrap();
    println!("PROST_COMPACT={}", prost_dynamic.to_text_format());
    println!("PROST_PRETTY={}", prost_dynamic.to_text_format_with_options(
        &FormatOptions::new().pretty(true)
    ));
}
```

Generate `metrics.pb` before the Rust run with `protoc --include_imports`
against `client_model` v0.6.1's `metrics.proto` and the matching
`google/protobuf/timestamp.proto`. This makes the comparison reproducible
without retaining scratch files in the repository.

The Rust probe lock contained 42 packages only because it deliberately compared
both rejected formatter stacks. Option A's wire seam adds no resolved runtime
package beyond the already-selected `prometheus[protobuf]` graph.

## Review

Fresh read-only adversarial contexts reviewed the draft against the oracle,
callers, dependency rules, primary sources, and probes. They found:

1. the zero-deviation claim omitted the reachable default Go/process families
   and their summary shape;
2. the first fixture was a raw protobuf message, not a length-delimited frame;
3. the proposed package-local deviation unnecessarily omitted timestamps from
   server-owned promhttp counters and incorrectly said framing bytes remained
   unchanged;
4. scratch-only probe paths were not durable evidence;
5. the single counter fixture did not prove the larger future acceptance matrix;
   and
6. the recorded compact/text hashes became stale after the probe binary changed,
   exposing protobuf-go's deliberate executable-dependent whitespace;
7. `cargo audit` evidence was incorrectly described as an audit of dependency
   unsafe code; and
8. "shorter frame prefix" conflated a smaller encoded length value with fewer
   prefix octets.

All findings were accepted and fixed. This record now inventories the 38
default families, makes their omission an explicit human decision, distinguishes
and probes raw versus delimited bytes, scopes Option B to DNS children, records
the consequent nested/frame length changes, embeds reconstruction inputs, and
labels the larger formatter matrix as an implementation acceptance obligation
rather than completed proof. The primary-source axis otherwise reported clean;
it also confirmed that Rust 1.96 compatibility for crates without a declared
MSRV is supported by the executable probe rather than attributed to an upstream
MSRV promise. The final finding was also accepted: the corrected observations
are embedded above, the upstream `detrand` behavior is primary-sourced, and the
options now require an explicit canonical-text deviation instead of promising
an impossible cross-build byte target. The last two wording findings were fixed
by limiting the security claim to advisory/path evidence and describing the
nested and outer length-value changes without claiming fewer uvarint octets.

Final independent read-only re-review after all fixes: `clean`; no actionable
finding remains.

Affected waiting package:

- `internal/machine/metrics` (`crates/ployz-internal-machine-metrics`)

Option A additionally affects the already-integrated prerequisite:

- `internal/metrics` (`crates/ployz-internal-metrics`)
