# Dependency decision: `go-compatible-json-codec`

| Field | Value |
| --- | --- |
| Status | `approved` by explicit user authority on `2026-08-12` |
| Capability | Go `encoding/json`-compatible JSON and incremental NDJSON codec for the reachable `internal/corrosion` and `internal/dns` contracts |
| Selected dependency | `serde = 1.0.229`, `serde_json = 1.0.151`, and `base64 = 0.22.1` for Go `[]byte` values |
| License | Direct crates: `MIT OR Apache-2.0`; the 12-package resolved dependency graph is permissive (MIT, Apache-2.0, Unlicense, and Unicode-3.0 expressions) |
| Research date | `2026-08-12` UTC |
| Request | [`migration/dependencies/requests/go-compatible-json-codec.md`](requests/go-compatible-json-codec.md) |

## Verdict

Approved for scheduling with the exact Serde stack below and small
protocol-specific visitors and formatters. It is the most popular and
idiomatic maintained Rust choice by
orders of magnitude, passes Rust 1.96 and all four required target checks, and
provides the natural typed, dynamic-value, raw-value, custom visitor, custom
formatter, and streaming APIs needed by the frozen packages.

Explicit user authority on 2026-08-12 accepts the two residual input differences
that are not reachable from the observed producers but are observable at a
generic codec boundary:

1. Go replaces invalid UTF-8 bytes and unpaired UTF-16 surrogate escapes when
   decoding into typed strings or dynamic values; `serde_json` rejects them at
   that typed decode boundary. `RawValue` capture can preserve an unpaired
   surrogate escape until a later destination scan.
2. Go permits nesting to 10,000 levels while `serde_json` retains its safer
   default recursion limit of 128. The frozen DNS and Corrosion shapes are
   shallow; this recommendation does not enable `unbounded_depth` or add a
   stack-growth dependency.

The selected stack does **not** natively reproduce all other Go behavior.
Approval must include the adapter obligations in this record. Those adapters
are bounded to the concrete DNS messages, Corrosion statements/events/admin
objects, and their byte/stream entry points. Do not build a generic Go API
imitation or a workspace-wide `encoding/json` compatibility facade.

If a future product requirement needs generic acceptance of malformed Unicode
or 129–10,000 levels of nesting, reopen research for a
bounded lossy-Unicode/depth implementation and its denial-of-service analysis.

## Primary-source evidence

### Frozen oracle and reachable callers

- DNS request encoding uses `json.NewEncoder(...).Encode`, so it is compact,
  HTML/JavaScript escaped, and terminated by exactly one newline. DNS response
  structs use `omitempty`, an embedded request struct, default unknown-field
  acceptance, and nested auth-error data:
  [`internal/dns/api.go`](../../upstream/uncloud/internal/dns/api.go) and
  [`internal/dns/client.go`](../../upstream/uncloud/internal/dns/client.go).
- Corrosion transaction and query bodies use `json.Marshal` without a trailing
  newline. Query responses are successive JSON values decoded from a reader.
  Row events are exact two-element arrays containing `uint64` plus raw column
  values, and scan defers typed decoding of each raw value:
  [`internal/corrosion/query.go`](../../upstream/uncloud/internal/corrosion/query.go).
- Subscription changes are exact four-element arrays containing a string,
  `uint64`, raw column values, and a `uint64` change ID. Subscriptions decode
  successive events incrementally and preserve decode-versus-server error
  boundaries:
  [`internal/corrosion/subscribe.go`](../../upstream/uncloud/internal/corrosion/subscribe.go)
  and its [stream fixtures](../../upstream/uncloud/internal/corrosion/subscribe_test.go).
- Admin frames decode into `map[string]any`; Go therefore turns every number
  into `float64`. Membership timestamps and RTT samples assert that dynamic
  numeric shape, including a timestamp beyond the exact-integer range of
  binary64:
  [`internal/corrosion/admin.go`](../../upstream/uncloud/internal/corrosion/admin.go)
  and [`admin_test.go`](../../upstream/uncloud/internal/corrosion/admin_test.go).
- The store supplies the concrete reachable parameter and scan types. In
  particular, domain JSON is stored as `[]byte`, and Corrosion BLOB values are
  scanned into `siteID` and `actorBytes` byte slices. Go encodes these as padded
  standard Base64 JSON strings, not integer arrays:
  [`internal/machine/store/store.go`](../../upstream/uncloud/internal/machine/store/store.go),
  [`container.go`](../../upstream/uncloud/internal/machine/store/container.go),
  and [`internal/machine/cluster/dns.go`](../../upstream/uncloud/internal/machine/cluster/dns.go).
- The remaining direct consumers use membership/admin results, subscription
  events, and DNS responses without comparing native JSON error text:
  [`internal/machine/machine.go`](../../upstream/uncloud/internal/machine/machine.go),
  [`internal/machine/cluster/cluster.go`](../../upstream/uncloud/internal/machine/cluster/cluster.go),
  and [`internal/machine/corroservice/service.go`](../../upstream/uncloud/internal/machine/corroservice/service.go).
- Repo-wide symbol tracing found no additional in-scope consumer of the public
  JSON event, admin response, or DNS wire types. `upstream/uncloud/experiment/**`
  was excluded as required by the controller instructions.

### Go `encoding/json` behavior

- Go v1 matches struct fields case-insensitively while preferring an exact
  match, processes members in input order, replaces or merges later duplicate
  values, maps untyped numbers to `float64`, distinguishes nil and non-nil empty
  slices, and ignores unknown struct fields by default: [Go decoder source and
  contract](https://go.dev/src/encoding/json/decode.go).
- Go rejects NaN and infinities, Base64-encodes byte slices, implements
  `omitempty`, sorts map keys, and escapes `<`, `>`, `&`, U+2028, and U+2029:
  [Go encoder source and contract](https://go.dev/src/encoding/json/encode.go).
- `Encoder.Encode` writes a newline after each value, while `Decoder.Decode`
  reads successive values from a stream: [Go stream
  source](https://go.dev/src/encoding/json/stream.go).
- Go uses a Unicode simple-fold field-name comparison, not only ASCII lower
  casing: [Go fold implementation](https://go.dev/src/encoding/json/fold.go).
  Every reachable frozen protocol field/tag is ASCII, so the required visitors
  need exact-preferred ASCII-insensitive matching for only those fixed names.

### Recommended exact stack

- `serde 1.0.229` declares `MIT OR Apache-2.0`, Rust 1.56, and the `derive` and
  `std` features in its [exact
  manifest](https://github.com/serde-rs/serde/blob/v1.0.229/serde/Cargo.toml).
  The derive feature resolves `serde_derive 1.0.229` in the probe lock; that
  release declares Rust 1.71 and the same license in its [exact
  manifest](https://github.com/serde-rs/serde/blob/v1.0.229/serde_derive/Cargo.toml).
- `serde_json 1.0.151` declares `MIT OR Apache-2.0`, Rust 1.71, and the selected
  `std`, `float_roundtrip`, and `raw_value` features in its [exact
  manifest](https://github.com/serde-rs/json/blob/v1.0.151/Cargo.toml).
- `float_roundtrip` selects the full-precision parsing path. The exact crate's
  tests record inputs for which default parsing is one ULP away from the
  correctly rounded value: [float round-trip
  fixtures](https://github.com/serde-rs/json/blob/v1.0.151/tests/test.rs#L957-L1008).
  The executable probe below confirmed that Go 1.26.1 and the selected feature
  produce identical bits for one such value while default `serde_json` does
  not.
- `RawValue` retains one valid JSON value's original text and can be used with
  reader-based deserialization, matching the deferred-decode need of
  `json.RawMessage`: [RawValue 1.0.151
  documentation](https://docs.rs/serde_json/1.0.151/serde_json/value/struct.RawValue.html).
- `StreamDeserializer` is an iterator over successive JSON values from a
  reader and exposes byte offsets: [StreamDeserializer 1.0.151
  documentation](https://docs.rs/serde_json/1.0.151/serde_json/struct.StreamDeserializer.html).
- `Serializer::with_formatter` and the public `Formatter` trait permit a
  bounded string/raw-fragment policy without forking the crate: [serializer
  source](https://github.com/serde-rs/json/blob/v1.0.151/src/ser.rs) and
  [Formatter 1.0.151
  documentation](https://docs.rs/serde_json/1.0.151/serde_json/ser/trait.Formatter.html).
- `base64 0.22.1` declares `MIT OR Apache-2.0`, Rust 1.48, and a `std` feature
  in its [exact
  manifest](https://docs.rs/crate/base64/0.22.1/source/Cargo.toml.orig).
  Its `STANDARD` engine is the padded RFC 4648 alphabet required by Go byte
  slices: [engine
  documentation](https://docs.rs/base64/0.22.1/base64/engine/general_purpose/constant.STANDARD.html).
  Version 0.22.1 is already selected elsewhere in this migration, had 482.2
  million exact-version downloads on the research date, and avoids resolving a
  second Base64 line solely for the newly released 0.23 API.

### Adoption, maintenance, and security snapshot

First-party crates.io values were read on 2026-08-12. Reverse-dependency counts
come from each crate's official `/reverse_dependencies?page=1&per_page=1` API.

| Crate | Current exact release | All-time downloads | Recent downloads | Reverse dependencies | Activity |
| --- | ---: | ---: | ---: | ---: | --- |
| `serde` | 1.0.229 | 1,257,445,571 | 261,525,550 | 114,047 | exact release updated 2026-07-18 |
| `serde_json` | 1.0.151 | 1,161,765,637 | 264,851,912 | 92,637 | exact release updated 2026-07-20; repository pushed 2026-08-08 |
| `base64` | 0.22.1 (0.23.1 is newest) | 1,423,695,648 crate-wide | 279,750,859 | 15,808 | 0.22.1 is the already-approved workspace line; exact release is non-yanked |
| `sonic-rs` | 0.5.8 | 4,937,503 | 1,707,429 | 176 | exact release updated 2026-03-25 |
| `simd-json` | 0.17.3 | 18,224,829 | 5,238,920 | 227 | exact release updated 2026-07-09 |
| `json` | 0.12.4 | 24,388,543 | 1,872,711 | 550 | last release updated 2020-03-18 |
| `miniserde` | 0.1.46 | 6,433,434 | 2,498,654 | 73 | exact release updated 2026-07-18 |

Sources: [`serde` crate
API](https://crates.io/api/v1/crates/serde), [`serde_json` crate
API](https://crates.io/api/v1/crates/serde_json), [`base64` crate
API](https://crates.io/api/v1/crates/base64), [`sonic-rs` crate
API](https://crates.io/api/v1/crates/sonic-rs), [`simd-json` crate
API](https://crates.io/api/v1/crates/simd-json), [`json` crate
API](https://crates.io/api/v1/crates/json), and [`miniserde` crate
API](https://crates.io/api/v1/crates/miniserde).

The selected exact probe lock was scanned against all 1,211 RustSec advisories
at advisory-db commit `d0861df1eab469d3c58d6b836ce48b5766e5f217` (database updated
2026-08-11). It reported zero vulnerabilities and no informational warnings.
This is advisory evidence for the resolved graph, not an audit of dependency
internals or a guarantee that no vulnerability exists.

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Required behavior | Compact Marshal, DNS newline Encode, Go escapes/`omitempty`, exact/folded fields, duplicate-last/merge, unknown fields, nil/null/empty distinctions, typed 64-bit ranges, untyped admin `float64`, raw tuples, Base64 bytes, and incremental events | Serde supplies all necessary typed/custom visitor/formatter/raw/stream hooks. The exact-version probes identify every non-native behavior and the obligations below constrain its adapter. Invalid Unicode rejection and the depth bound are explicitly accepted limitations rather than silent claims. | `pass` |
| Losslessness and numbers | Full reachable `i64`/`u64`; Go-correct binary64 parsing; raw column values; Go dynamic-number coercion; reject non-finite output | Typed Serde integers cover the target's 64-bit `int`/`uint`; `float_roundtrip` matched Go bits; `RawValue` retains column input; a dedicated admin visitor must coerce numbers to `f64`; non-finite output must be rejected before Serde's null fallback. | `pass pending required adapters` |
| Field, escaping, and errors | Exact-preferred case-insensitive fixed fields; later duplicates overwrite/merge; Go HTML/JS escaping; caller-visible error boundaries | Custom visitors and `Formatter` are public natural Serde APIs. Exact native error wording differs but no caller compares it; package wrappers must retain syntax/data/EOF/I/O and server-error boundaries. | `pass pending required adapters` |
| Platforms and targets | Linux/macOS amd64/arm64 on Rust 1.96; no external runtime | The exact selected graph and the broader comparison graph both passed `cargo check` for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`. All selected crates are Rust libraries with no C/system dependency. | `pass` |
| Maintenance and Rust version | Maintained exact releases compatible with Rust 1.96 | Serde/serde_json were updated in July 2026 and declare MSRV at most 1.71. Base64 0.22.1 declares 1.48, is non-yanked, is already approved in the migration, and compiled in the exact graph. | `pass` |
| License and security | Permissive licenses; no known advisory in exact graph; bounded untrusted-input posture | Direct crates are MIT OR Apache-2.0; resolved licenses are permissive. RustSec was clean at the recorded database commit. Keep serde_json's recursion limit and do not enable unsafe fast-path alternatives. | `pass` |
| Architecture and build cost | Idiomatic Rust APIs; bounded adapters; no Go facade, unsafe FFI, SIMD requirement, or unnecessary second stack | Serde visitors/formatters let the concrete protocol types own their semantics. The exact selected probe resolved 12 third-party packages, including derive-time crates; only Serde/serde_json/base64 are direct. | `pass` |
| Popularity | Prefer the most widely adopted passing exact-version stack | Serde/serde_json lead the credible alternatives by roughly two orders of magnitude in reverse dependencies and far more in downloads; established projects including Deno use `serde_json::Value` in production code. | `pass` |
| Human authority | Newly discovered exact crates/features and material residuals must be explicitly accepted | No `go-compatible-json-codec` row exists in `migration/AUTHORITIES.tsv`; the request explicitly forbids inferred approval. | `fail until human decision` |

## Candidate comparison

| Candidate | Behavior and architecture | Maintenance, portability, and adoption | Decision |
| --- | --- | --- | --- |
| **`serde 1.0.229` + `serde_json 1.0.151`** | Natural strongly typed, dynamic `Value`, raw-value, custom visitor/formatter, and reader-stream APIs. Native `Value` exact duplicates are last-wins. Defaults still need the bounded Go policy below. Safe public API and no input mutation. | Rust 1.56/1.71 MSRVs; current July 2026 releases; Linux/macOS architecture-neutral graph; 1.16B+ downloads and 92,637 reverse dependencies for serde_json. | **Selected and approved.** Only candidate with leading adoption, simple architecture, and every required extension point. |
| `sonic-rs 0.5.8` | Serde-compatible SIMD JSON stack, but derived structs need the same case/duplicate/null adapters. Its dynamic value preserves duplicate members rather than Go's last-value map. The differential probe also emitted unescaped Go-sensitive characters, integer-array bytes, and null for NaN. It adds a sizable unsafe SIMD implementation for small control-plane messages. | Apache-2.0; no declared MSRV, though it compiled on Rust 1.96 and all required targets; active in 2026; 4.94M downloads and 176 reverse dependencies. Its own [compatibility guide](https://github.com/cloudwego/sonic-rs/blob/e339451e37a8f2d3f9b93f155985d31fcbcb115a/docs/serdejson_compatibility.md) documents differences. | Rejected on fidelity, architecture, build cost, and popularity. No measured workload justifies the added SIMD/unsafe surface. |
| `simd-json 0.17.3` | Serde path still resolves serde_json, parsing mutates an input buffer, native `OwnedValue` preserves duplicates, and no simpler incremental reader model results. The probe found the same escaping/bytes/field/null gaps and its serializer rendered NaN as `2.696539702293474e308`, which is not Go's required error. | MIT OR Apache-2.0; Rust 1.88; active July 2026; supports x86_64/aarch64 but deliberately contains extensive SIMD unsafe code; 18.2M downloads and 227 reverse dependencies. A historical high-severity unsafe advisory is fixed in current versions: [RUSTSEC-2019-0008](https://rustsec.org/advisories/RUSTSEC-2019-0008.html). | Rejected on behavior and architecture. It adds mutation/SIMD complexity and a second Serde-backed stack without a parity benefit. |
| `miniserde 0.1.46` | Supports a smaller typed JSON model, but its own README calls it a prototype rather than a production-quality artifact, supplies only `rename`, has no fallible serialization or detailed errors, and directs custom use cases to Serde: [exact README](https://github.com/dtolnay/miniserde/blob/0.1.46/README.md). Those limitations prevent the required visitors, formatters, raw tuple policy, and I/O stream behavior. | MIT OR Apache-2.0; Rust 1.71; current release but only 73 reverse dependencies. | Rejected at behavior and architecture gates. |
| `json 0.12.4` | Standalone dynamic JSON without Serde derives, RawValue, or the required incremental reader/custom typed integration. | MIT/Apache-2.0; last released in March 2020; no declared MSRV; 550 reverse dependencies. | Rejected at maintenance and architecture gates. |
| `json-deserializer 0.4.4` / `jiter 0.16.0` | Parser-oriented alternatives lack a complete serializer + RawValue + typed reader-stream stack. Jiter's native DOM also preserves duplicates. | The former was last released in 2022; Jiter is active but had only 11 reverse dependencies in the first-party snapshot. | Rejected at completeness and adoption gates. |

The popularity preference is decisive only after hard gates. Sonic-rs and
simd-json compile on the targets, but neither passes the relevant behavior and
architecture gates more naturally than Serde. Miniserde and the older dynamic
parsers fail before popularity is considered.

## Selected integration

Use these exact direct dependency settings.
Root workspace manifests and the lockfile remain integrator-owned.

```toml
base64 = { version = "=0.22.1", default-features = false, features = ["std"] }
serde = { version = "=1.0.229", default-features = false, features = ["derive", "std"] }
serde_json = { version = "=1.0.151", default-features = false, features = ["float_roundtrip", "raw_value", "std"] }
```

Do not enable `serde_json/arbitrary_precision`: Go admin `any` values use
binary64 rather than arbitrary precision. Do not enable `preserve_order`: the
default sorted object map is closer to Go's sorted map emission. Do not enable
`unbounded_depth`: the safer 128-level bound is an explicit pending limitation.
Do not add Sonic, simd-json, Miniserde, a fork, or an application `unsafe` fast
path under this decision.

The implementation should use Serde's natural traits and concrete Rust wire
models. It must not expose Go-shaped `Marshal`, `Unmarshal`, `RawMessage`,
`Encoder`, or `Decoder` APIs merely to imitate the oracle.

### Required common policy

1. Provide separate compact-write operations for the two observed modes:
   Corrosion `Marshal` equivalents write no newline; the DNS request body
   equivalent appends exactly one `\n` only after successful serialization.
2. Use a custom `serde_json::ser::Formatter` that escapes `<`, `>`, and `&` as
   lower-case `\u00xx`, and U+2028/U+2029 as `\u2028`/`\u2029`, for object keys
   and values. Retain Serde's compact separators and standard control escapes.
3. Do not serialize `RawValue` verbatim where Go would marshal a `RawMessage`.
   Go validates, compacts, and HTML/JS-escapes raw fragments while preserving
   number lexemes. Implement a bounded validated raw-fragment compactor/escaper
   or an equivalent event serializer, and pin it with tuple fixtures containing
   whitespace, Go-sensitive characters, and noncanonical number spellings.
4. Reject NaN and positive/negative infinity before Serde can convert them to
   `null`. The current callers do not send floating-point parameters, but this
   retains the reachable statement type's error boundary.
5. Handwrite visitors for the fixed protocol structs whose incoming field
   behavior is observable. Process members in input order; prefer an exact
   fixed-name match, then an ASCII-insensitive match; ignore unknown fields;
   assign later scalar/slice/pointer values over earlier values. When a repeated
   nested struct is decoded, preserve Go's merge behavior for fields absent in
   the later object rather than blindly replacing the whole object.
6. Model missing, explicit `null`, nil, non-nil empty, and populated containers
   only where the frozen API distinguishes them. A fresh zero struct may use
   `#[serde(default)]` only when it is behaviorally identical. Use explicit
   `skip_serializing_if` predicates for Go `omitempty`; do not assume
   `Option<T>` alone distinguishes missing from null.
7. Preserve caller-visible failure stages and package context: reader I/O,
   clean EOF, unexpected EOF, JSON syntax/type/range failures, Corrosion
   server-error fields, tuple arity failures, and DNS response wrappers. Native
   serde_json message wording and byte columns need not match because no caller
   observes or compares them.

### `internal/corrosion` obligations

- Use a protocol parameter/value enum rather than `serde_json::Value` as the
  general SQL parameter API. It must encode the reachable null, bool, signed
  64-bit, unsigned 64-bit, finite binary64, UTF-8 string, padded standard Base64
  byte string, and validated raw JSON cases with Go semantics.
- Model byte slices separately from ordinary `Vec<u8>`. Non-nil `[0,255]`
  encodes as `"AP8="`; non-nil empty bytes encode as `""`; nil bytes encode as
  `null`. Scanning must preserve the null/empty distinction needed by concrete
  destinations. Go's `base64.StdEncoding` ignores only embedded CR/LF and is
  non-strict about unused trailing bits. Before decoding, remove only `\r` and
  `\n`; then use a `GeneralPurpose` engine with `alphabet::STANDARD`, canonical
  padding, and `GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true)`.
  Reject every other invalid byte and malformed or missing padding. Do not use
  the stricter prebuilt `STANDARD` constant for incoming Go byte strings.
- Use `Box<RawValue>` or a reader-lifetime raw value for deferred column decode.
  Decode each `Rows::scan` / change scan destination independently so the first
  failing column retains its index and earlier destinations remain assigned, as
  in the oracle.
- Handwrite two- and four-element tuple visitors. Reject shorter or longer
  arrays and retain the oracle's distinct row/type/row-ID/values/change-ID error
  stages.
- Integrate incrementally with the approved asynchronous HTTP/2 body rather
  than calling synchronous `std::io::Read` parsing on a Tokio runtime worker.
  Retain a bounded partial-value buffer across response-body chunks and attempt
  `Deserializer::from_slice`/stream parsing as bytes arrive. If implementation
  instead requires a blocking bridge, return to the dependency gate for that
  architecture and its cancellation/resource ownership approval. Never buffer
  the entire subscription or block a runtime worker. Preserve distinct
  fragmented-value, clean EOF, truncated EOF, body I/O error, cancellation,
  and body-drop outcomes; cancellation/body drop must promptly unblock the
  owning task without an orphaned codec task.
- For admin objects, use a dedicated Go-like dynamic visitor or recursively
  normalize every number to `f64` before field parsing. Exact duplicate object
  keys are last-wins. In particular, membership `ts` must follow Go's binary64
  rounding before conversion to `u64`; RTT arrays must reject non-numbers.
- Keep the default recursion limit. The admin protocol's known objects and
  query events are far below 128 levels.

### `internal/dns` obligations

- Give `DomainResponse`, `RecordRequest`, `RecordResponse`,
  `AuthErrorResponse`, and nested auth data their exact JSON names. Preserve the
  embedded request fields in `RecordResponse` without introducing an extra
  object.
- Reproduce `omitempty` for empty name/type/values, zero status, empty message,
  and false `noDomain`. The `data` member has no `omitempty` and must encode as
  an object even when all nested fields are empty.
- Accept fixed field names case-insensitively with exact preference, ignore
  unknown response fields, apply later duplicates/merges, and implement Go's
  null-as-no-op behavior for non-pointer scalar/nested struct targets. Preserve
  nil versus non-nil empty `values` if the response is re-exposed or encoded.
- Produce a compact request body with Go escaping and exactly one trailing
  newline. Do not add a newline to Corrosion bodies as a side effect of sharing
  this policy.

## Accepted limitations and risks

| Risk or deviation | Scope and mitigation |
| --- | --- |
| Invalid UTF-8 and unpaired surrogate input | At typed string/dynamic-value decode boundaries, Go replaces malformed string data with U+FFFD while serde_json returns a syntax/data error. `RawValue` capture may preserve an unpaired surrogate escape until a Corrosion destination scan, where typed string/value decoding rejects it. The frozen Corrosion and DNS producers use valid UTF-8 and no caller fixture relies on replacement. Explicitly accept this input-hardening difference or reopen the decision. |
| Nesting limit | Go v1 accepts up to 10,000 levels; serde_json defaults to 128. All observed wire shapes are shallow. Retaining 128 reduces stack-exhaustion risk; values deeper than 128 are an accepted rejection difference only after human authority. |
| Generic outbound float text | Even with correct input rounding, Rust and Go can choose different finite float spellings (`1.0` versus `1`, exponent thresholds). No reachable caller sends a floating-point Corrosion parameter and DNS has no float fields. Tests must reject non-finite values and cover any future float caller before use. |
| Raw fragment re-encoding | `RawValue` serializes verbatim, unlike Go's validated compact/escape path. The required raw-fragment adapter and fixtures are an acceptance gate, not optional cleanup. |
| Visitor correctness | Exact-preferred fold matching, duplicate order, nested merge, and null/container state are easy to get subtly wrong. Each concrete struct requires differential fixtures rather than a generic derive claim. |
| Resource bounds | Neither Go nor Serde imposes package-specific byte/body limits here. The dependency decision does not invent one. Existing HTTP/frame policy owns any transport bound; retain Serde's depth bound. |
| Exact native error text | Serde and Go messages/offsets differ. Current callers preserve only success/failure and add package context. If a future caller compares native text, return to the dependency gate. |

No approved behavior deviation permits losing 64-bit IDs, coercing BLOBs to
integer arrays, accepting non-finite output as JSON null, buffering a live
subscription, dropping tuple raw values, or silently changing DNS's newline and
escaping bytes.

## Verification commands and probes

The oracle packages passed in their reproduced toolchain:

```sh
cd upstream/uncloud
mise exec -- go version
# go version go1.26.1 linux/amd64
mise exec -- go test ./internal/corrosion ./internal/dns
# ok internal/corrosion; internal/dns has no test files
```

The Go 1.26.1 differential probe and exact Rust candidate probe established:

```text
Go Encoder string:       "\u003c\u003e\u0026\u2028\u2029" + newline
Serde/Sonic/simd string:  literal <>& U+2028 U+2029
Go []byte{0,255}:         "AP8="
all three Rust defaults:  [0,255]
Go StdEncoding decode:    "/\r\nw==" and noncanonical "/x==" -> ff
base64 STANDARD decode:   rejects both forms
selected scan adapter:    strips CR/LF, accepts "/x==", rejects missing "/w"
Go NaN:                   serialization error
serde_json / sonic-rs:    null
simd-json 0.17.3:         2.696539702293474e308
Go folded field:          NAME populates name
all three Rust derives:   NAME is unknown/defaulted
Go duplicate known field: last wins
all three Rust derives:   duplicate-field error
Go null Vec:              nil; [] is distinct non-nil empty
all three Rust derives:   null type error
Go untyped duplicate map: last wins
serde_json Value:         last wins
sonic-rs Value:           preserves both duplicate members
simd-json OwnedValue:     preserves both duplicate members
Go/selected float bits:   51.248178375505404 -> 0x40499fc44f1b2f60
serde_json default bits:  51.248178375505404 -> 0x40499fc44f1b2f61
Go typed invalid surrogate:     U+FFFD, success
serde_json typed surrogate:     error; RawValue can defer typed rejection
Go typed invalid UTF-8 string:  U+FFFD, success
serde_json typed invalid UTF-8: error
Go and serde_json stream: both decoded two successive NDJSON objects
```

The exact selected scratch graph used Rust 1.96 with the dependency lines in
this record. It asserted `u64::MAX`, `RawValue`, two successive reader values,
`STANDARD` Base64 encoding `AP8=`, the custom Go-compatible decoding settings
for embedded CR/LF, nonzero unused bits, and required padding, and the
Go-matching float bits, then passed:

```sh
cargo fmt --manifest-path /tmp/ployz-selected-json-probe/Cargo.toml --check
cargo test --locked --manifest-path /tmp/ployz-selected-json-probe/Cargo.toml
cargo clippy --locked --manifest-path /tmp/ployz-selected-json-probe/Cargo.toml \
  --all-targets -- -D warnings
cargo check --locked --manifest-path /tmp/ployz-selected-json-probe/Cargo.toml \
  --target x86_64-unknown-linux-gnu
cargo check --locked --manifest-path /tmp/ployz-selected-json-probe/Cargo.toml \
  --target aarch64-unknown-linux-gnu
cargo check --locked --manifest-path /tmp/ployz-selected-json-probe/Cargo.toml \
  --target x86_64-apple-darwin
cargo check --locked --manifest-path /tmp/ployz-selected-json-probe/Cargo.toml \
  --target aarch64-apple-darwin
cargo audit --file /tmp/ployz-selected-json-probe/Cargo.lock
```

The broader exact-version comparison graph containing serde_json 1.0.151,
sonic-rs 0.5.8, and simd-json 0.17.3 also checked for all four target triples.
Scratch paths are not durable artifacts. The durable reconstruction inputs are
the versions/features, fixture/result matrix, primary-source links, and commands
above.

Package acceptance must add pinned differential fixtures for:

- every DNS zero/nonzero `omitempty` combination and the required final newline;
- exact, ASCII case variants, exact-plus-folded duplicates, unknowns, nested
  duplicate merge, missing/null/empty/populated fields;
- byte nil/empty/`[0,255]`, embedded Base64 CR/LF, a Go-accepted nonzero-unused-
  trailing-bit spelling, malformed/missing padding and other invalid Base64,
  and end-to-end Store put/scan;
- row/change tuple arity and raw values containing whitespace, `<>&`,
  U+2028/U+2029, and noncanonical number lexemes;
- `i64::MIN`, `i64::MAX`, `u64::MAX`, overflow/fractional-to-integer failures,
  admin binary64 rounding, and NaN/infinities;
- blank-line NDJSON, concatenated values, every response-chunk split within a
  value, fragmented EOF, malformed middle events, body I/O errors,
  cancellation/body close, bounded partial buffering, and proof that no Tokio
  runtime worker is blocked; and
- the accepted invalid-Unicode and 128/129 nesting boundary differences.

## Authority resolution

The user selected the recommended bounded stack on 2026-08-12, exactly:

```text
serde 1.0.229: default-features=false + [derive,std]
serde_json 1.0.151: default-features=false + [float_roundtrip,raw_value,std]
base64 0.22.1: default-features=false + [std]
```

Invalid-Unicode rejection and the 128-level nesting limit are accepted only for the
documented frozen DNS/Corrosion boundaries, and require every adapter and
acceptance fixture in this record. No generic Go `encoding/json` compatibility
runtime is authorized.

No option authorizes registry/root-manifest edits by this dependency owner.
The controller owns the registry/authority update and the integrator owns
workspace dependency resolution.

## Review

Fresh read-only primary research independently reached the same stack, feature
set, adapter obligations, affected callers, and the initial
`human-decision-required` status before user approval. A separate fresh
adversarial dependency reviewer reported two findings:

- `R01` (blocking): synchronous `from_reader` guidance did not define a safe
  integration with the async HTTP/2 subscription. Fixed by requiring bounded
  chunk-fed incremental parsing, prohibiting runtime-worker blocking, and
  making a blocking bridge a new dependency-gate architecture decision.
- `R02` (non-blocking): the invalid-surrogate limitation incorrectly implied
  rejection at every serde_json boundary. Fixed by distinguishing typed
  string/dynamic decode from `RawValue` capture and deferred scan rejection.
- `R03` (non-blocking, independent re-review): the incoming byte-scan guidance
  treated the prebuilt `base64::STANDARD` decoder as equivalent to Go's more
  tolerant `base64.StdEncoding`. Fixed by requiring removal of only CR/LF, a
  custom standard-alphabet engine that allows unused trailing bits while still
  requiring canonical padding, and differential fixtures for both tolerances
  and malformed input.

The independent reviewer rechecked R01, R02, R03, and the complete corrected
record against the exact base, oracle, callers, request, registries, source
evidence, and probes. Final result: **CLEAN — no actionable finding remains.**

Affected waiting packages:

- `internal/corrosion`
- `internal/dns`
