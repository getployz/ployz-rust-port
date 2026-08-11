# Package packet: `internal/corrosion`

## Assignment

| Field | Value |
| --- | --- |
| Go package | `upstream/uncloud/internal/corrosion` |
| Migration crate | `ployz-internal-corrosion` |
| Owned path | `crates/ployz-internal-corrosion/**` |
| Base commit | `565209151fc54d459bb6ff8de6f4c3793faedc7e` |
| Wave | `0` |
| State | `catalogued` |

The implementor owns only the path above. The integrator owns root workspace files. The controller owns this packet and registries.

## Oracle inventory

### Package files

Direct production sources (all ordinary, ungenerated Go files with no build constraints):

- `upstream/uncloud/internal/corrosion/admin.go` — Unix admin-socket framing, response decoding, membership parsing, NTP conversion, and RTT statistics.
- `upstream/uncloud/internal/corrosion/client.go` — API client construction, authentication, transport retry, and the legacy `RetrySubscription` wrapper.
- `upstream/uncloud/internal/corrosion/query.go` — transaction and query HTTP protocols, row-event wire format, and streaming row cursor.
- `upstream/uncloud/internal/corrosion/subscribe.go` — subscription HTTP protocol, change-event wire format, change streaming, and resubscription.

Direct package tests:

- `upstream/uncloud/internal/corrosion/admin_test.go` — five RTT-statistic cases and nine RTT-response parsing cases.
- `upstream/uncloud/internal/corrosion/client_test.go` — bearer-header insertion without request mutation and empty-token behavior.
- `upstream/uncloud/internal/corrosion/subscribe_test.go` — zero-change resubscription snapshot draining, nonzero resubscription, and terminal 404 behavior.

External-package tests are empty. Generated files are empty. Platform-suffixed files and build-tagged files are empty. The package nevertheless has a runtime Linux/Unix constraint: its admin client connects to a Unix-domain socket. There are no embedded assets or package-local fixtures; the subscription tests construct their exact newline-delimited JSON streams inline.

### Direct callers

- `upstream/uncloud/internal/machine/machine.go:247-259` constructs the API and admin clients, gives the API client to `store.Store`, and gives the admin client to `cluster.Cluster`. Both constructors are expected to be non-I/O setup; startup wraps any constructor error.
- `upstream/uncloud/internal/machine/corroservice/service.go:24-63` constructs the API client and repeatedly calls `QueryContext("SELECT 1 FROM cluster LIMIT 1")` as the schema-readiness probe; a successful HTTP/query setup is sufficient and rows need not be iterated.
- `upstream/uncloud/internal/machine/store/store.go:25-333` stores an `*APIClient`; it depends on `ExecContext` result/error behavior, `QueryContext`, `Rows.Next/Scan/Err/Close`, initial subscription rows, `Subscription.Changes/Err/ID`, ordered row values, raw JSON conversion into strings/integers/bytes, and change-channel closure.
- `upstream/uncloud/internal/machine/store/container.go:55-288` calls the API client held by `Store`; it depends on `ExecContext.RowsAffected`, variadic SQL parameters, `QueryContext`, five-column row scanning, initial subscription rows, and subscription change/closure signaling.
- `upstream/uncloud/internal/machine/cluster/cluster.go:198-241,272-275` depends on latest-per-ID membership state selection, exact `Alive`/`Suspect` state strings, parsed member IP addresses, fatal propagation of any admin error, and RTT address/duration statistics.

No other Go file imports this package, and no other file invokes a corrosion client through a containing field. The exported `WithHTTP2Client`, `WithResubscribeBackoff`, `ExecMultiContext`, `ResubscribeContext`, event codecs, `Rows.Columns/Time`, raw `SendCommand`, and `RetrySubscription` surface have no production caller in the frozen tree; they remain accounted for below because they are observable package API.

### Exported Go surface

- `admin.go`: `AdminClient`, `NewAdminClient`, `Response`, `AdminClient.SendCommand`, `ClusterMembershipState`, `MembershipStateAlive`, `MembershipStateSuspect`, `MembershipStateDown`, `AdminClient.ClusterMembershipStates`, `MemberRTTStats`, and `AdminClient.ClusterMemberRTTs`.
- `client.go`: `APIClient`, `NewAPIClient`, `APIClientOption`, `WithHTTP2Client`, `WithResubscribeBackoff`, `AuthRoundTripper`, `AuthRoundTripper.RoundTrip`, `RetryRoundTripper`, `RetryRoundTripper.RoundTrip`, `RetrySubscription`, `NewRetrySubscription`, `RetrySubscription.ID`, `RetrySubscription.Changes`, `RetrySubscription.Err`, and `RetrySubscription.Close`.
- `query.go`: `Statement`, `ExecResponse`, `ExecResult`, `APIClient.ExecContext`, `APIClient.ExecMultiContext`, `QueryEvent`, `EndOfQuery`, `RowEvent`, `RowEvent.UnmarshalJSON`, `RowEvent.MarshalJSON`, `APIClient.QueryContext`, `Rows`, `Rows.Columns`, `Rows.Next`, `Rows.Err`, `Rows.Scan`, `Rows.Time`, and `Rows.Close`.
- `subscribe.go`: `ChangeType`, `ChangeTypeInsert`, `ChangeTypeUpdate`, `ChangeTypeDelete`, `ErrSubscriptionNotFound`, `ChangeEvent`, `ChangeEvent.UnmarshalJSON`, `ChangeEvent.MarshalJSON`, `ChangeEvent.Scan`, `Subscription`, `Subscription.ID`, `Subscription.Rows`, `Subscription.Changes`, `Subscription.Err`, `Subscription.Close`, `APIClient.SubscribeContext`, and `APIClient.ResubscribeContext`.

### Imports and external capabilities

Internal Go imports are empty: this is a wave-0 leaf package.

The standard-library imports are `bytes`, `context`, `crypto/tls`, `encoding/binary`, `encoding/json`, `errors`, `fmt`, `io`, `log/slog`, `math`, `net`, `net/http`, `net/netip`, `net/url`, `sort`, `strconv`, and `time`. Tests additionally use `net/http/httptest`, `sync/atomic`, and `testing`.

Non-standard production imports are `github.com/cenkalti/backoff/v4` and `golang.org/x/net/http2`. Test-only `github.com/stretchr/testify` is Go-oracle infrastructure and does not imply a Rust dependency. The corresponding Rust capability requests are listed in the dependency gate below without selecting crates.

## Behavior contract

| ID | Input or event | Required result | Errors, ordering, timing, or limitation | Evidence |
| --- | --- | --- | --- | --- |
| B01 | Construct an API client from an IP socket address and bearer token. | Use base URL `http://<AddrPort>` and a cleartext prior-knowledge HTTP/2 transport. Default TCP connect timeout is 3 seconds. Install bearer authentication, per-request transport retry, and subscription-resubscribe retry. Construction performs no network I/O. | Default transport retry starts at 100 ms and stops after 2 seconds elapsed. Default resubscribe retry starts at 100 ms, caps intervals at 1 second, and stops after 60 seconds elapsed. IPv6 address formatting must retain brackets through URL construction. | `client.go:19-86`; callers `machine.go:247-250`, `corroservice/service.go:38-41` |
| B02 | Supply client options. | Apply options in argument order after defaults. A custom HTTP client replaces the whole default client; a custom resubscribe-backoff factory is used for each resubscription sequence. | Replacing the HTTP client bypasses built-in authentication and transport retry. A nil resubscribe factory disables automatic resubscription in `Subscription`; it is distinct from passing a nil backoff to `NewRetrySubscription`, which selects that wrapper's default. | `client.go:82-104` |
| B03 | Send through the authentication transport. | With a nonempty token, clone the request, set `Authorization: Bearer <token>` on the clone, and leave the caller's request/header unchanged. With an empty token, pass the original request through without adding or removing the header. | `Set` replaces any existing Authorization values on the clone. | `client.go:106-120`; `client_test.go:12-60` |
| B04 | An HTTP transport attempt returns an error or response. | Retry only errors in the Go `net.OpError` family using a fresh backoff per request, bounded by the request context; return other errors immediately and return any HTTP response immediately regardless of status. | Retries reuse the same request and body without rewinding it, so a partially consumed request body is not guaranteed to replay correctly. HTTP status codes are never retried here. | `client.go:122-146` |
| B05 | Execute one or more SQL statements. | POST JSON to `/v1/transactions` with `Content-Type` and `Accept` set to `application/json`. The multi payload is an array of `{query,params}` in caller order; a nil parameter slice encodes as `null`. The single form wraps exactly one statement, returns the first result, and exposes `rows_affected`, result time, response time, and optional version. | Request creation, serialization, send, body-read, and decode failures retain their stage-specific context. `ExecContext` errors with `no results` if a successful multi response has no results. Schema changes are not made cluster-synchronizable by this API. | `query.go:13-69`, `query.go:30-53` |
| B06 | `/v1/transactions` returns HTTP 200. | Decode one `ExecResponse`. Return the decoded response even when results contain database errors, and return the ordered join of every non-null result error. | Invalid JSON is `decode response`. Multiple database errors remain distinguishable only through the joined error; successful result entries do not suppress failing ones. | `query.go:71-85` |
| B07 | `/v1/transactions` returns HTTP 500 or another non-200. | For 500, read the body and try to decode `ExecResponse`; if its first result has an error, return that error alone. Otherwise report `internal server error: <raw body>`. For any other status, report `unexpected status code <code>: <raw body>`. | On 500, only the first result's error is inspected; later errors are ignored. A body read error replaces status/body reporting. The response body is closed on every completed execution path. | `query.go:86-105` |
| B08 | Execute a row-returning SQL query. | POST one `{query,params}` object to `/v1/queries` with JSON content/accept headers. HTTP 200 hands the open streaming body to the row cursor; any other status reads/closes the body and reports code plus raw body. | Success requires parsing the first stream event before returning. Cancellation and transport errors retain stage context. | `query.go:149-189`; caller `corroservice/service.go:43-52` |
| B09 | Encode or decode a row event. | Its wire form is exactly JSON array `[row_id, values]`; require exactly two elements, an unsigned 64-bit row ID, and an array of raw JSON values. Marshal back to the same shape without interpreting value payloads. | Wrong JSON shape, length, ID, or values type is rejected as `invalid row event`. | `query.go:123-147` |
| B10 | Create a row cursor from a query stream. | If context is already cancelled, return its cancellation error without decoding. Otherwise synchronously decode one event and require a non-null `columns` array; store its names and position the cursor before the first row. An empty columns array is valid. | The first event is accepted whenever `columns` is non-null, even if other event fields are also populated. `Columns` returns the held collection rather than a defensive copy in Go, so caller mutation is observable. | `query.go:191-233` |
| B11 | Advance a row cursor. | Check context first; then decode one JSON event. A server `error` is fatal. A `row` is yielded only when its value count exactly matches the column count. An `eoq` stores time/change ID, yields false, and closes the body only for standalone queries. | Error precedence is decode, server error, row, EOQ, then unexpected event. Context, decode, server, arity, and unexpected-event failures close the body and set `Err`. Subscription snapshot cursors deliberately leave the body open at EOQ. | `query.go:235-283` |
| B12 | Scan the current row into destinations. | Reject a prior cursor error and destination-count mismatch; otherwise JSON-decode each raw value into the corresponding destination in column order. | There is no explicit before-first/after-last state check; behavior follows the currently stored row (initially empty). A later column failure can leave earlier destinations mutated. Scan errors do not become `Rows.Err`. | `query.go:285-308`; store callers in `store.go:34-48,65-87,97-116,133-177,180-223,270-333` and `container.go:105-177,213-288` |
| B13 | Ask for query time or close rows. | `Time` returns EOQ seconds only after EOQ. Before EOQ it reports the existing cursor error if any, otherwise `time is not available until all rows are consumed`. `Close` delegates to the response body and does not change `Err`. | API documentation promises idempotent close; the implementation does no separate idempotence bookkeeping and inherits the concrete body's repeated-close behavior. Callers may close without consuming rows. | `query.go:310-327`; `corroservice/service.go:43-52` |
| B14 | Encode or decode a change event. | Its wire form is exactly JSON array `[type,row_id,values,change_id]`; require exactly four elements, string type, unsigned 64-bit IDs, and raw JSON values. Marshal to the same shape. | The codec accepts arbitrary type strings; `insert`, `update`, and `delete` are named values, not validation constraints. | `subscribe.go:17-61` |
| B15 | Scan a change event. | Destination count must equal value count; decode raw JSON values into destinations in order. | A later failure may leave earlier destinations mutated. No type-specific conversion beyond JSON decoding is added. | `subscribe.go:63-77` |
| B16 | Create a subscription. | POST `{query,params}` to `/v1/subscriptions`; add `skip_rows=true` only when requested; set JSON content/accept headers; require HTTP 200 and nonempty `corro-query-id`. With `skip_rows=false`, synchronously consume the columns event and expose a `Rows` cursor sharing the stream decoder. With `skip_rows=true`, expose no rows and treat the body as an immediate change stream. | Non-200 reads/closes the body and includes raw body in the error. Missing ID and initial snapshot-parse errors close the body. Skip-rows subscriptions start with unknown change ID 0, so their first event is not continuity-checked. | `subscribe.go:227-286` |
| B17 | Inspect a subscription and start changes. | `ID` returns the response ID; `Rows` returns the initial cursor or nil for skipped rows/resubscriptions. `Changes` is idempotent after first success and returns the same channel. If initial rows exist, all must reach EOQ before changes are available; initialize last change ID from EOQ. | Calling before EOQ returns an error and starts nothing. The Go implementation dereferences EOQ `change_id` without checking for null; an EOQ without `change_id` panics when `Changes` is called. Concurrent calls are unsynchronized and unsupported. | `subscribe.go:117-153` |
| B18 | Consume subscription change events. | Decode and deliver events serially. Server errors, absent change events, malformed JSON, and gaps are stream errors. Once last ID is nonzero, require exactly `last+1`; update last ID before delivery. Closing/cancelling closes the body to unblock decoding, closes the output stream, and suppresses cancellation-caused decoder errors. | When last ID is zero, any first change ID is accepted. Backpressure is one unbuffered handoff. Without automatic resubscribe, a stream error becomes `Err`; explicit/context close leaves `Err` unchanged. | `subscribe.go:128-225` |
| B19 | A subscription stream fails with automatic resubscription enabled. | Retry GET resubscription from the last delivered/accepted change ID using a new bounded backoff. On success replace the body/decoder and continue the same output stream. A 404 is permanent, is attempted exactly once, closes the output stream, and leaves an error that matches `ErrSubscriptionNotFound`. | Other failures retry until success, cancellation, or elapsed-time exhaustion; terminal error is wrapped as `resubscribe to query with backoff`. The old response body is not explicitly closed when replaced after a non-cancellation stream error. | `subscribe.go:191-212,288-313`; `subscribe_test.go:161-215` |
| B20 | Explicitly resubscribe by ID and change ID. | GET `/v1/subscriptions/<id>?from=<decimal-u64>` with JSON content/accept headers and no `skip_rows`. HTTP 404 returns the stable not-found sentinel after closing the body; other non-200 responses include code/raw body. | The `corro-query-id` response header is not required on resubscription. | `subscribe.go:315-367` |
| B21 | A successful resubscription starts from change 0 versus nonzero. | From 0, synchronously parse and drain the replayed columns/rows/EOQ snapshot, expose no rows, retain the same decoder/body, then stream changes. From nonzero, decode changes directly without expecting or skipping a snapshot. | This is compatibility with Corrosion v1.0.0+ behavior. Snapshot parse/drain failures close the body and are contextualized. The first post-zero-snapshot change is not continuity-checked because the new subscription's last ID remains zero. | `subscribe.go:348-367`; `subscribe_test.go:46-159` |
| B22 | Use the separately exported legacy `RetrySubscription` wrapper. | Preserve its observable frozen behavior and mark it deprecated/internal-to-the-port: it exposes the underlying ID, calls underlying `Changes`, records last IDs, and intends to resubscribe on closure with caller backoff or a 100 ms/1 second/60 second default. | Frozen flaw: its output channel is never allocated. `Changes` returns a nil stream and nil error, can never forward an event, and its worker panics by closing the nil channel when cancellation or a terminal path makes it return. It has no frozen-tree caller; do not accidentally present it as a working retry abstraction. | `client.go:148-256`; repository-wide symbol search finds no caller |
| B23 | Construct an admin client and send a raw command. | Construction stores the socket path, performs no I/O or validation, and always succeeds. Sending synchronously connects to that Unix socket and writes one frame: 4-byte big-endian unsigned payload length followed by exact command bytes. Only after a successful write does it return a response stream. | Connect/write failures are immediate and contextualized. There is no context, deadline, message-size bound, peer authentication, or non-Unix fallback. Payload lengths above `u32::MAX` cannot be represented faithfully. | `admin.go:16-43,96-121`; `machine.go:252-255` |
| B24 | Read framed admin responses. | Read exactly four header bytes and exactly the declared body, decode arbitrary JSON, and loop. String `"Success"` ends successfully without emission. Other strings/types are ignored. A map containing `Error` as an object emits one terminal error (its string `msg`, else invalid-error-response); otherwise a map containing `Json` as an object emits that object and continues. | `Error` takes precedence if both recognized keys exist. Unknown maps are ignored. JSON/frame/EOF failure emits one terminal error. Connection and stream close after success or the first error. A malicious length can cause allocation up to the 32-bit declared size. | `admin.go:31-94,96-121` |
| B25 | Request cluster membership states. | Send exact command bytes `{"Cluster":"MembershipStates"}`. Parse each JSON response's nested `id.id`, nested `id.addr`, top-level state, and nested `id.ts`. Accept state strings exactly `Alive`, `Suspect`, or `Down`. Parse socket addresses including bracketed IPv6. Convert the 64-bit fixed-point timestamp as Unix-relative seconds plus floored `(fraction*1e9)>>32` nanoseconds. | Admin response errors return immediately because the producer then closes. JSON decoding represents `ts` as float64 before conversion, so large 64-bit values can lose precision; this upstream limitation is observable. Missing/wrong fields and unknown states are errors. | `admin.go:123-217` |
| B26 | Select all or latest membership states. | With `latest=false`, retain every valid state in response order. With `latest=true`, retain one per ID and replace it only for a strictly later timestamp; equal timestamps keep the first. Drain all responses after parse errors, join all parse errors, and return valid states together with the joined error. | Latest-mode output order is unspecified/nondeterministic because Go map iteration is used. The cluster caller treats any returned parse error as fatal and does not use partial states. | `admin.go:217-260`; caller `cluster.go:198-241` |
| B27 | Request and parse member RTTs. | Send exact command bytes `{"Cluster":"Members"}`. Require `state.addr`. Missing or null `rtts` means no samples; an empty array also means no emitted member. Require every sample to be a JSON number. Preserve response order among emitted members and return valid results alongside joined parse errors. | Wrong state/address/RTT shapes are accumulated while the response stream is drained. The cluster caller returns any error and discards partial results. | `admin.go:268-303,335-373`; `admin_test.go:67-174`; caller `cluster.go:272-275` |
| B28 | Compute RTT statistics for a nonempty sample set. | Sort numerically, choose the middle for odd counts or average the two middle values for even counts, and compute population (divide by N) standard deviation. Convert floating-point milliseconds to durations by Go's float-to-integer duration conversion. | Empty input returns zero/zero internally but callers omit empty sample sets. Samples are not range-, sign-, infinity-, or NaN-validated. Sorting mutates the provided sample slice. | `admin.go:289-332`; `admin_test.go:12-65` |
| B29 | Callers consume API/admin objects over time. | Keep request/event order, cancellation responsiveness, body/socket closure, and error identity/wrapping described above. The streaming row and subscription handles are single-consumer mutable cursors; the API client itself can initiate independent requests concurrently. | Do not promise thread-safe concurrent mutation of a single `Rows`, `Subscription`, or legacy `RetrySubscription`; the Go types contain unguarded mutable state. | `query.go:193-203`; `subscribe.go:80-92`; `client.go:148-158` |

## Rust design freedom

The Rust crate need not reproduce Go names, option functions, channels, goroutines, pointer-null conventions, or file/module boundaries. It must expose an idiomatic client/stream API sufficient for the direct callers while preserving the protocol bytes, event ordering, retry/cancellation boundaries, stable error distinctions (especially subscription-not-found), partial-result semantics, timing bounds, and documented limitations above.

The external constraints are the Corrosion HTTP/2 and Unix-socket protocols, request/response JSON shapes, `corro-query-id` header, exact admin commands and length framing, SQL parameter ordering, membership/RTT meanings, and direct-caller needs. The Rust implementation may use async streams, iterators, enums, typed values, and structured errors shaped by approved dependencies. If a deliberate Rust API does not expose an unused Go symbol directly, its behavior row must still be covered by a characterization test or an explicit compatibility/deprecation decision; B22's frozen defect must not be silently transformed into a supported working API.

## Dependency capabilities

`migration/DEPENDENCIES.tsv` contains no approved decisions at this base, so every non-standard capability is research-required and this packet cannot become `ready` yet.

| Capability | Decision record | Status |
| --- | --- | --- |
| Cleartext prior-knowledge HTTP/2 client with streaming response bodies, injectable test transport/server support, headers, request-body uploads, per-request cancellation, and a 3-second TCP connect bound | `migration/dependencies/http2-client.md` | `research-required` |
| Async/concurrent I/O runtime and cancellation primitives for HTTP streams, Unix-domain sockets, backpressure, and prompt close/unblock semantics on Linux | `migration/dependencies/async-runtime.md` | `research-required` |
| JSON serialization/deserialization including raw-value preservation, arbitrary values/SQL parameters, exact array event codecs, and incremental decoding of consecutive top-level JSON documents | `migration/dependencies/json.md` | `research-required` |
| Exponential-backoff retry with fresh policies, elapsed-time and max-interval bounds, permanent-error classification, and cancellation-aware waits | `migration/dependencies/retry-backoff.md` | `research-required` |

Big-endian framing, numeric statistics, IP socket parsing, error aggregation, and Unix-domain sockets are available in Rust/platform facilities, but a decision may fold their implementation into one of the capabilities above if primary-source research shows that is the idiomatic design. Do not choose or add a crate until the corresponding decision is approved.

## Test traceability

The required Rust test names below belong in the owned crate. `tests/go_oracle.rs` must use the exact inline JSON streams from the three Go test files where referenced; source-only rows need focused characterization fixtures. Result remains pending until implementation/review.

| Behavior ID | Go test or source evidence | Required Rust test | Result |
| --- | --- | --- | --- |
| B01 | `client.go:19-86` | `go_oracle::api_client_defaults_and_ipv6_base_url` | `pending` |
| B02 | `client.go:82-104` | `go_oracle::client_options_apply_in_order_and_replace_defaults` | `pending` |
| B03 | `client_test.go:12-60` | `go_oracle::auth_header_and_request_immutability` | `pending` |
| B04 | `client.go:122-146` | `go_oracle::transport_retries_only_network_errors_without_status_retry` | `pending` |
| B05 | `query.go:13-69` | `go_oracle::transaction_request_shape_and_single_result_selection` | `pending` |
| B06 | `query.go:71-85` | `go_oracle::transaction_200_returns_response_and_all_db_errors` | `pending` |
| B07 | `query.go:86-105` | `go_oracle::transaction_non_200_error_precedence_and_body_text` | `pending` |
| B08 | `query.go:149-189`; `corroservice/service.go:43-52` | `go_oracle::query_request_status_and_initial_event` | `pending` |
| B09 | `query.go:123-147` | `go_oracle::row_event_exact_array_codec` | `pending` |
| B10 | `query.go:205-233` | `go_oracle::rows_require_columns_and_honor_pre_cancel` | `pending` |
| B11 | `query.go:235-283` | `go_oracle::rows_event_precedence_arity_eoq_and_close` | `pending` |
| B12 | `query.go:285-308`; store callers | `go_oracle::rows_scan_count_raw_json_and_partial_mutation` | `pending` |
| B13 | `query.go:310-327` | `go_oracle::rows_time_and_close_lifecycle` | `pending` |
| B14 | `subscribe.go:17-61` | `go_oracle::change_event_exact_array_codec_and_open_type` | `pending` |
| B15 | `subscribe.go:63-77` | `go_oracle::change_scan_count_and_partial_mutation` | `pending` |
| B16 | `subscribe.go:227-286` | `go_oracle::subscribe_request_skip_rows_id_and_snapshot_setup` | `pending` |
| B17 | `subscribe.go:117-153` | `go_oracle::changes_gate_idempotence_and_missing_change_id_limitation` | `pending` |
| B18 | `subscribe.go:155-225` | `go_oracle::change_stream_order_gap_backpressure_cancel_and_error` | `pending` |
| B19 | `subscribe_test.go:161-215` | `go_oracle::resubscribe_not_found_is_one_attempt_and_terminal` | `pending` |
| B20 | `subscribe.go:315-347` | `go_oracle::resubscribe_request_and_status_mapping` | `pending` |
| B21 | `subscribe_test.go:46-159` | `go_oracle::resubscribe_zero_drains_snapshot_nonzero_streams_changes` | `pending` |
| B22 | `client.go:148-256`; no callers | `go_oracle::legacy_retry_subscription_nil_channel_defect` | `pending` |
| B23 | `admin.go:16-43,96-121` | `go_oracle::admin_unix_command_frame_and_immediate_failures` | `pending` |
| B24 | `admin.go:31-94` | `go_oracle::admin_response_variants_precedence_and_termination` | `pending` |
| B25 | `admin.go:123-217` | `go_oracle::membership_parse_states_address_and_unix_ntp` | `pending` |
| B26 | `admin.go:217-260`; `cluster.go:198-241` | `go_oracle::membership_all_latest_partial_errors_and_order` | `pending` |
| B27 | `admin_test.go:67-174` | `go_oracle::member_rtt_parse_missing_empty_invalid_and_partial` | `pending` |
| B28 | `admin_test.go:12-65` | `go_oracle::rtt_median_and_population_stddev_cases` | `pending` |
| B29 | mutable-state definitions in all production files | `go_oracle::independent_requests_and_single_consumer_stream_contract` | `pending` |

## Acceptance commands

Run from the repository root. The crate is intentionally not added to the root workspace by its implementor; these commands target its owned manifest directly.

```sh
cargo fmt --manifest-path crates/ployz-internal-corrosion/Cargo.toml -- --check
cargo check --manifest-path crates/ployz-internal-corrosion/Cargo.toml --all-targets
cargo test --manifest-path crates/ployz-internal-corrosion/Cargo.toml --all-targets
cargo clippy --manifest-path crates/ployz-internal-corrosion/Cargo.toml --all-targets --all-features -- -D warnings
```

Run the frozen targeted Go oracle exactly with its pinned toolchain:

```sh
(
  cd upstream/uncloud
  mise exec -- go test -count=1 -run '^(TestComputeRTTStatsMs|TestParseClusterMemberRTT|TestAuthRoundTripper_SetsAuthorizationHeader|TestAuthRoundTripper_EmptyTokenSkipsHeader|TestSubscription_ResubscribeFromZeroDrainsSnapshot|TestSubscription_ResubscribeFromNonZeroSkipsSnapshot|TestSubscription_ResubscribeNotFoundFailsFast)$' ./internal/corrosion
)
```

Run the paired Go-oracle/Rust fixture differential. The Rust `go_oracle` integration target must carry the exact Go test streams/cases plus source-characterization cases for B01-B29; both halves must pass in the same environment.

```sh
(
  cd upstream/uncloud
  mise exec -- go test -count=1 -run '^(TestComputeRTTStatsMs|TestParseClusterMemberRTT|TestAuthRoundTripper_SetsAuthorizationHeader|TestAuthRoundTripper_EmptyTokenSkipsHeader|TestSubscription_ResubscribeFromZeroDrainsSnapshot|TestSubscription_ResubscribeFromNonZeroSkipsSnapshot|TestSubscription_ResubscribeNotFoundFailsFast)$' ./internal/corrosion
) && cargo test --manifest-path crates/ployz-internal-corrosion/Cargo.toml --test go_oracle -- --nocapture
```

Linux is required for `go_oracle::admin_unix_command_frame_and_immediate_failures`. No privileged service or live Corrosion process is required: use loopback HTTP fixtures and a temporary Unix-domain socket. For a live-protocol smoke test when Corrosion infrastructure is supplied, the reviewer must additionally verify h2c transaction/query/subscription interoperability and admin framing; absence of that optional external service does not replace the deterministic protocol fixtures above.

## Handoff

Behavior questions: none unresolved. Packet blockers: none. Readiness gates: all four dependency capability decisions above are missing and must be approved before the controller may move this packet to `ready`.

The implementor records its commit, deliberate behavior mappings (including the explicit treatment of unused B22), and command results here. Reviewers use `migration/REVIEW_TEMPLATE.md`; the controller alone records state changes and blockers.
