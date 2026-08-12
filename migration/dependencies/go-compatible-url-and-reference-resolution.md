# Dependency decision: Go-compatible URL and reference resolution

| Field | Value |
| --- | --- |
| Status | `approved` by explicit user authority on `2026-08-12` |
| Capability | Go `net/url`-compatible endpoint parsing and redirect reference resolution for `internal/dns` |
| Selected dependency | `oxiri = 0.3.1`, exact, no crate features, plus the smallest package-owned compatibility seam required for observable DNS behavior |
| Recommended candidate | `oxiri = 0.3.1`, exact, no crate features |
| Candidate license | `MIT OR Apache-2.0` |
| Research date | 2026-08-12 UTC |
| Request | [`migration/dependencies/requests/go-compatible-url-and-reference-resolution.md`](requests/go-compatible-url-and-reference-resolution.md) |
| Exact integration base | `16bc253ab172` |

No unmodified Rust crate passed the complete observable behavior gate. The
approved option is exact `oxiri 0.3.1` plus a
bounded package-owned Go compatibility seam. `oxiri` preserves the important
RFC spelling and percent-encoded-dot behavior, has a two-crate pure-Rust
production graph, forbids unsafe code in `oxiri` itself, and passed strict
audit and target gates.
Its parser is deliberately stricter than Go in some components, and its
resolver differs for empty and query/fragment-only references over an
unnormalized base, so the seam is a material architecture choice. Explicit user
authority is recorded in [`migration/AUTHORITIES.tsv`](../AUTHORITIES.tsv),
releasing `internal/dns` from this dependency gate.

## Oracle boundary

The frozen implementation is
[`upstream/uncloud/internal/dns/client.go`](../../upstream/uncloud/internal/dns/client.go).
It constructs two targets by raw string concatenation before parsing:

```text
endpoint + "/domains"
endpoint + "/domains/" + domain + "/records"
```

Both strings then pass to Go 1.26.1 `http.NewRequest`; `http.DefaultClient`
owns redirects. The endpoint is supplied by `uc dns reserve --endpoint` or
`uc machine init --dns-endpoint`, crosses the cluster API unchanged, and is
persisted for later record creation. The relevant callers were traced through:

- [`cmd/uc/dns/reserve.go`](../../upstream/uncloud/cmd/uc/dns/reserve.go),
  including the default `https://dns.uncloud.run/v1`;
- [`cmd/uc/machine/init.go`](../../upstream/uncloud/cmd/uc/machine/init.go);
- [`internal/machine/cluster/dns.go`](../../upstream/uncloud/internal/machine/cluster/dns.go),
  which stores and later reuses `Endpoint`, `Name`, and `Token`;
- [`pkg/client/dns.go`](../../upstream/uncloud/pkg/client/dns.go),
  [`cmd/uc/caddy/deploy.go`](../../upstream/uncloud/cmd/uc/caddy/deploy.go),
  and [`cmd/uc/machine/add.go`](../../upstream/uncloud/cmd/uc/machine/add.go),
  which reach record creation/update.

There are no frozen `internal/dns` tests. Repository-wide symbol search,
excluding the out-of-scope `upstream/uncloud/experiment/**`, found no custom
`CheckRedirect`, cookie jar, URL helper, or alternate DNS HTTP client. The
required contract is the narrow Go 1.26.1 behavior below, not every `net/url`
API.

### Endpoint construction and request target

- Concatenate the suffix before parsing. An existing `?` or `#` changes where
  the suffix lands: `https://h/v1#frag` yields wire target `/v1` and fragment
  `frag/domains`; `https://h/v1?x=1` sends `/v1?x=1/domains`.
- Go parsing lowercases the scheme. `http.NewRequest` removes an empty explicit
  port (`HtTp://EXAMPLE.test:/...` becomes scheme `http`, host
  `EXAMPLE.test`). Otherwise preserve host case, explicit nonempty default
  ports, repeated slashes, and literal dot segments at construction.
- Path parsing decodes to `Path` while retaining a valid non-default spelling
  in `RawPath`; serialization preserves valid raw path escape spelling and
  escapes raw spaces, UTF-8 bytes, and backslashes. Backslash is data (`%5C`),
  never a separator. Escaped slash and escaped dot remain encoded.
- Percent behavior is component-specific, not global. Malformed escapes fail
  where Go invokes component unescaping/validation, including hierarchical
  path and fragment. `RawQuery` is stored without unescaping or validation,
  and opaque scheme content returns before path validation, so
  `https://h/p?q=%zz` and `foo:bad%zz` construct successfully.
- Authority spelling has separate rules. Userinfo is decoded and reserialized,
  so `us%65r:pa%73s` becomes `user:pass`; percent-encoded ASCII host spelling
  such as `%65xample.test` is rejected under host-specific restrictions.
  Preserve accepted URL userinfo. Go transport
  may derive Basic authorization from it for reserve; an explicit record
  bearer header takes precedence.
- Fragment parsing similarly retains a valid non-default `RawFragment` spelling
  but fragments never enter the request target. Raw query does, without form
  decoding or re-encoding.
- Absolute non-HTTP schemes may construct and fail only at transport. The seam
  must preserve this error phase and observable error behavior where relevant.

Primary sources are the frozen call sites above, Go 1.26.1
[`net/url`](https://github.com/golang/go/blob/go1.26.1/src/net/url/url.go),
[`http.NewRequest`](https://github.com/golang/go/blob/go1.26.1/src/net/http/request.go),
and [`http.Client`](https://github.com/golang/go/blob/go1.26.1/src/net/http/client.go).
Go documents RFC 3986 behavior with compatibility deviations; the baseline
resolution algorithm is [RFC 3986 section 5.2](https://www.rfc-editor.org/rfc/rfc3986#section-5.2).

### Redirect resolution and policy

For each nonempty `Location` followed by `http.DefaultClient`:

- parse it as a reference and resolve it against the immediately preceding
  request URL;
- inherit or replace scheme, authority (including userinfo), path, query, and
  fragment according to reference form; an authority reference with no
  userinfo clears inherited userinfo;
- remove literal dot segments while preserving repeated slashes and
  percent-encoded dot segments;
- normalize literal dot segments already present in the base even for empty,
  query-only, or fragment-only references; an empty reference preserves the
  base fragment;
- never send the resolved fragment;
- change POST to GET for 301/302/303 and drop the body plus
  `Content-Encoding`, `Content-Language`, `Content-Location`, and
  `Content-Type`;
- retain method and replayable body for 307/308. The record body is a
  `bytes.Buffer`, so `http.NewRequest` supplies `GetBody`; reserve has no body;
- retain sensitive headers only when the destination is the same
  IDNA-canonicalized hostname or a subdomain of the initial hostname. The test
  ignores scheme and port. The exact set is `Authorization`,
  `Www-Authenticate`, `Cookie`, `Cookie2`, `Proxy-Authorization`, and
  `Proxy-Authenticate`. Once a cross-domain hop strips them, they remain
  stripped. This client has no cookie jar;
- add `Referer` except on HTTPS-to-HTTP downgrade and omit URL userinfo from it;
- return a 3xx response with empty `Location` without following, fail on a
  malformed nonempty `Location`, and stop before request 11 with
  `stopped after 10 redirects`.

This loop is package policy over the separately approved raw HTTP stack. A URL
crate alone cannot satisfy it. Reqwest's built-in redirect policy is not
selected because it would duplicate that stack and couple the package to
WHATWG URL behavior.

## Executable golden comparison

Bounded probes used Go 1.26.1 and Rust 1.96.0. The initial shared probe
compared `url 2.5.8`, `fluent-uri 0.4.1`, `uriparse 0.6.4`, and
`iri-string 0.7.14`; exact standalone probes then exercised `iref 4.2.0` and
`oxiri 0.3.1`. Claims below are limited to cases actually run for each crate.

| Case | Go 1.26.1 | `url 2.5.8` | `oxiri 0.3.1` | Other RFC candidates |
| --- | --- | --- | --- | --- |
| initial `https://EXAMPLE.test:443/...` | preserve host case and `:443` | lowercase host; remove `:443` | preserve | preserve |
| initial `/a/../b/./c//...` | preserve dot segments and repeated slash | eagerly normalize dots | preserve | preserve |
| mixed-case scheme and empty port | lowercase scheme; remove empty port | normalized | preserves both; seam needed | preserves; seam needed |
| raw space or `\` in path | escape as `%20` or `%5C` | space escaped; `\` becomes `/` | reject; seam needed | reject; seam needed |
| `%zz` in hierarchical path | reject | accept unchanged | reject | reject |
| `%zz` in raw query or opaque content | accept unchanged | accepts tested query | reject; component seam needed | generally reject; seam needed |
| escaped userinfo / encoded host | normalize userinfo; reject encoded host | WHATWG normalization | preserves accepted spelling / accepts encoded host; seam needed | component-specific mismatches |
| resolve `/a//b/../c` | `/a//c` | same | same | same |
| resolve `%2e%2e/x` | preserve encoded dot segment | treats it as `..` | **same as Go** | `iref` matches; tested `iri-string`, `fluent-uri`, and `uriparse` treat it as `..` |
| resolve `..\x` | keep backslash data as `..%5Cx` | treats it as separator | reject; seam needed | reject; seam needed |
| empty ref with base fragment | preserve fragment | preserves after prior WHATWG normalization | clears fragment; seam needed | `iref` also clears; other resolver-specific mismatch |
| empty/query/fragment ref over base literal dots | normalize base path | base already normalized | preserves base dots; seam needed | resolver-specific mismatch |

The Go redirect probe additionally observed:

| Redirect | Second request |
| --- | --- |
| same-host 302 from POST | GET, no body or `Content-Type`; bearer, cookie, and inherited userinfo retained |
| subdomain absolute 302 with no target userinfo | bearer and cookie retained; inherited userinfo cleared |
| cross-domain 302 | GET; sensitive and body headers stripped |
| cross-domain 307 | POST and body replayed; `Content-Type` retained; sensitive headers stripped |

These are future package-test gates, not permission to add a dependency. Probe
artifacts are outside the repository:

```text
/tmp/ployz-url-probe
  go_probe.go        0edd734b6a2a915659e540f04a53c5a15a3045e671e4920d41a1db985d55ddb0
  precision_probe.go 6fcfa8d37eb0eb0c30b166e7e12acfe647e75c1debc5189981249bd678cffecd
  redirect_probe.go  e84859566ddcd3adb97e991ea363c1e694b1d4018e21bb87a7c4690f479caa0f
  Cargo.toml          da2ae7cd40e5152a31b81451cf6a37f19feb85e0e9cb60cba8603930da34f3a8
  Cargo.lock          8e9fcb143dcf1dac6f4c9b5725e97af66567c4b1e49a54e48343f304a352cd40
  src/main.rs         ed557182c17e2999b6518a85421f5bb94f1bf5e49721bec9ec7be9b7be806719

/tmp/ployz-oxiri-candidate
  Cargo.toml          af4082cdb3bb849a4c5d06cd4bd533bba197d76edd0149b59ce6c9abb2856a31
  Cargo.lock          ee35e0531f62258645c605437edc2e8fc6a3d867856884a57b2a518975144a3b
  src/main.rs         70cf623d8cbf1bf6699d30fbe240a3d6fa8747700c442d311947f9b4f4542e7e

/tmp/ployz-iri-candidate
  Cargo.toml          42764d14683d18a5ee1cf8c33ba3a48a2b2cfbbb61e7fe22aeba0325561f661c
  Cargo.lock          a8698d292377fa0ba73797077effa3e545cd1c55e6524ee34e03265c02a2a0fe
  src/main.rs         3ac6a2b56688ac6c860201a452d01bd609c82aeee8df569e30fac491fddb9706

/tmp/ployz-iref-candidate
  Cargo.toml          8b7575a1c6caf4d6ce7211bf2e2d2a4f8a9539b443f0ee16016017cc8a4fe87d
  Cargo.lock          2eb411274d3d1496bdf43b708980f40f1cef6f4470f934cd92744554c82e2f30
  src/main.rs         eebf77a669682d68c7322f4a5e3cf503b8cc686b50f0a09b1f223d75681f79bb
```

## Hard gates

| Gate | Requirement | Evidence | Result |
| --- | --- | --- | --- |
| Behavior | Exact concatenation, component-specific Go escaping/errors/normalization, resolution, fragments, credentials, and redirect policy | Every unmodified candidate has a material mismatch. `oxiri` has the narrowest clean-graph mismatch set; its narrow package-owned seam is explicitly authorized and remains package-tested acceptance work. | `pass with approved seam` |
| License/security | Permissive, safe integration, no advisory or maintenance warning | `oxiri 0.3.1` is MIT OR Apache-2.0; resolved `memchr 2.8.3` is Unlicense OR MIT. `cargo audit --deny warnings` loaded 1,211 advisories and exited 0. `oxiri` has `#![deny(unsafe_code)]`; the selected `memchr` graph is pure Rust. | pass for exact graph |
| Platforms | Linux/macOS, amd64/arm64 | Exact candidate checked under Rust 1.96 for all four requested targets; Linux amd64 probe ran. No native build or OS API. | pass |
| Maintenance/Rust | Maintained, MSRV <= 1.96 | `oxiri 0.3.1` was released 2026-07-25, repository active in 2026, declared MSRV 1.83. Run/test/Clippy passed on Rust 1.96. | pass |
| Architecture | Natural Rust IRI/reference types and bounded seam; no unapproved deviation | `Iri`/`IriRef` are natural validated types. Component compatibility, final HTTP URI conversion, and redirect policy remain package-owned under the explicit authority row. | `pass` |

Overall verdict: **human decision required**. No dependency may be consumed
from this record as written.

## Candidate comparison

Official crates.io metadata was queried 2026-08-12. “Dependents” means the
crates.io reverse-dependency total, not audited production use. Behavior and
security are hard gates; popularity ranks candidates only after those gates.

| Candidate | Version / license / MSRV | Adoption and maintenance | Result |
| --- | --- | --- | --- |
| `url` | 2.5.8; MIT OR Apache-2.0; Rust 1.63 | 792,695,384 total / 168,656,318 recent downloads; 14,531 dependents; active 2026 | Rejected. Overwhelmingly popular and idiomatic for WHATWG URLs, but eager host/default-port/dot/backslash normalization and permissive percent behavior violate this contract. `Url::join` cannot restore discarded spelling. |
| `iri-string` | 0.7.14; MIT OR Apache-2.0; Rust 1.60 | 197,537,375 / 53,264,532; 72 dependents; released 2026-07-26 | Clean, maintained runner-up. Preserves initial spelling and has a tiny graph, but deliberately recognizes encoded dot segments during resolution, widening the seam beyond `oxiri`. |
| `fluent-uri` | 0.4.1; MIT; Rust 1.68 | 59,490,686 / 18,129,250; 58 dependents; active 2026 | Rejected. Safe, maintained RFC types, but rejects Go raw inputs, recognizes encoded dots, refuses a base fragment, and offers no parity advantage. |
| `uriparse` | 0.6.4; MIT; undeclared MSRV | 30,483,465 / 3,815,008; 70 dependents; last release 2022-03-18 | Rejected for stale maintenance and no behavior advantage. |
| `iref` | 4.2.0; legacy manifest `MIT/Apache-2.0` with both license files; Rust 1.89 | 6,830,376 / 987,679; 95 dependents; released 2026-07-07 | Rejected by strict maintenance/security gate. It matches encoded-dot behavior but its 21-package third-party graph (22 lock entries including probe root) includes unmaintained `proc-macro-error 1.0.4`, RUSTSEC-2024-0370, through `static-automata-macros 1.0.3` and `str-newtype-derive 3.0.0`; `cargo audit --deny warnings` fails. |
| `oxiri` | **0.3.1**; MIT OR Apache-2.0; Rust 1.83 | 1,334,205 / 675,682; 52 dependents; released 2026-07-25 | **Selected and approved.** It preserves initial spelling and encoded-dot segments, exposes natural IRI/reference types, forbids unsafe code, and resolves through only `memchr`. Its stricter component validation and empty/base-dot reference behavior require the documented seam. |
| `ada-url` | 4.0.0; MIT OR Apache-2.0; undeclared MSRV | 140,375 / 77,352; 5 dependents; active 2026 | Rejected. WHATWG mismatch class, bundled C++/FFI default, and much lower Rust adoption. |
| `http::Uri` from approved HTTP stack | 1.5.0; MIT OR Apache-2.0 | Existing Hyper-family primitive | Useful only after compatible spelling is fixed. It is strict and has no reference resolver. |
| Reqwest redirect stack | 0.13.4; MIT OR Apache-2.0 | Popular maintained client | Rejected. It duplicates the selected raw HTTP stack, uses WHATWG URL semantics, and hides policy this package must control. |

Primary records: [`oxiri 0.3.1`](https://crates.io/crates/oxiri/0.3.1) and
[source](https://github.com/oxigraph/oxiri/tree/v0.3.1),
[`url 2.5.8`](https://crates.io/crates/url/2.5.8),
[`iri-string 0.7.14`](https://crates.io/crates/iri-string/0.7.14),
[`fluent-uri 0.4.1`](https://crates.io/crates/fluent-uri/0.4.1),
[`uriparse 0.6.4`](https://crates.io/crates/uriparse/0.6.4),
[`iref 4.2.0`](https://crates.io/crates/iref/4.2.0), and
[`ada-url 4.0.0`](https://crates.io/crates/ada-url/4.0.0).

## Approved integration

Exact direct dependency and feature policy:

```toml
oxiri = { version = "=0.3.1", default-features = false }
```

Exact resolved production graph from the probe lockfile:

| Crate | Version | Enabled features | License | Role |
| --- | --- | --- | --- | --- |
| `oxiri` | `0.3.1` | none (`serde` disabled) | MIT OR Apache-2.0 | checked `Iri`/`IriRef` parsing and RFC resolution |
| `memchr` | `2.8.3` | `default`, `std`, `alloc` | Unlicense OR MIT | parser byte search; resolved transitive version, not a new direct dependency |

The package should own a narrow request-target builder that uses checked
`Iri`/`IriRef` APIs for the RFC-compatible hierarchical core, not expose a
Go-shaped URL type:

1. concatenate the oracle suffix before component parsing;
2. lex the Go component boundaries and reproduce component-specific behavior:
   lowercase scheme, remove empty explicit port at the `NewRequest` boundary,
   normalize userinfo, enforce host escapes, preserve `RawPath`/
   `RawFragment` spelling when valid, leave `RawQuery` and opaque spelling
   unvalidated, and apply Go path escaping without altering accepted host case,
   nonempty ports, slashes, or literal dots;
3. pass only the `oxiri`-compatible hierarchical core/reference subset through
   checked constructors. Carry Go-accepted raw query and opaque spelling in
   package-owned fields beside that core; never force invalid `%` through
   `oxiri` or use its unchecked constructors;
4. strip fragments only when producing the HTTP request target;
5. resolve redirects with `oxiri` where it matches, while package tests pin
   empty-reference fragment preservation, base-path dot cleanup for empty/
   query/fragment-only references, and Go backslash escaping;
6. build final `http::Uri` only after compatible spelling is fixed; and
7. own method/body/header/referrer/ten-hop redirect policy in the HTTP client
   module with the golden matrix as mandatory tests.

Do not add logging beyond the oracle. Go redacts the current URL password in
some errors but quotes malformed `Location` in another; error and redaction
parity belong in goldens. Dependency display strings are not stable package
API.

### Residual risks

- The seam is not implemented. Add goldens for IPv6 zones, Unicode/IDNA hosts,
  empty query markers, encoded userinfo, opaque schemes, malformed redirect
  components, authority edge cases, and error precedence before acceptance.
- `oxiri` is an RFC IRI crate, not a Go compatibility crate. Keep the exact
  pin and rerun differentials before any update.
- The parser accepts Unicode IRI text; wire conversion still needs explicit
  Go-compatible URI escaping and hostname policy.
- URL userinfo and subdomain-sensitive redirect copying carry credential risk.
  Keep this seam internal to the frozen DNS client contract.
- Cross-target results are compile checks; runtime behavior ran on Linux amd64.
- The broader HTTP transport decision is separate; this record neither expands
  nor repairs it.

## Authority resolution

On 2026-08-12 the user selected exact `oxiri 0.3.1`, no crate features, with
the smallest package-owned seam needed for observable Uncloud DNS behavior.
The target is idiomatic Rust; unobservable Go implementation quirks are not a
compatibility requirement. This does not authorize `iri-string`, a generic
package-owned URL runtime, WHATWG `url`, or `iref`.

## Verification commands and results

```text
git rev-parse --short=12 HEAD
  16bc253ab172

/opt/go1.26.1/bin/go run /tmp/ployz-url-probe/go_probe.go
/opt/go1.26.1/bin/go run /tmp/ployz-url-probe/precision_probe.go
  PASS: construction, component-percent, normalization, and reference goldens

/opt/go1.26.1/bin/go run /tmp/ployz-url-probe/redirect_probe.go
  PASS: same/subdomain/cross-domain 302 and cross-domain 307 observations

(cd /tmp/ployz-url-probe && cargo +1.96.0 run --locked)
  PASS: shared url/fluent-uri/uriparse/iri-string comparison

(cd /tmp/ployz-oxiri-candidate && cargo +1.96.0 run --locked)
  PASS: exact oxiri construction/reference probe

(cd /tmp/ployz-oxiri-candidate && \
  cargo +1.96.0 test --locked --all-targets --all-features && \
  cargo +1.96.0 clippy --locked --all-targets --all-features -- -D warnings)
  PASS: exact consumer graph

(cd /tmp/ployz-oxiri-candidate && cargo audit --file Cargo.lock --deny warnings)
  PASS: 1,211 advisories loaded; no vulnerability or maintenance warning

(cd /tmp/ployz-oxiri-candidate && \
  cargo +1.96.0 check --locked --target <target>)
  PASS: x86_64-unknown-linux-gnu
  PASS: aarch64-unknown-linux-gnu
  PASS: x86_64-apple-darwin
  PASS: aarch64-apple-darwin

cargo +1.96.0 test --manifest-path \
  ~/.cargo/registry/src/index.crates.io-*/oxiri-0.3.1/Cargo.toml --all-features
  PASS: 13 integration tests and 31 doctests

(cd /tmp/ployz-iri-candidate && \
  cargo +1.96.0 test --locked --all-targets --all-features && \
  cargo +1.96.0 clippy --locked --all-targets --all-features -- -D warnings && \
  cargo audit --file Cargo.lock)
  PASS: exact iri-string consumer tests/Clippy/audit

(cd /tmp/ployz-iref-candidate && \
  cargo +1.96.0 test --locked --all-targets --all-features && \
  cargo +1.96.0 clippy --locked --all-targets --all-features -- -D warnings)
  PASS: exact iref consumer tests/Clippy

(cd /tmp/ployz-iref-candidate && cargo audit --file Cargo.lock --deny warnings)
  EXPECTED FAIL: RUSTSEC-2024-0370, proving the strict gate failure
```

## Review

Fresh adversarial researcher `/root/adversarial_record_review` reviewed the
pre-final record read-only against Go 1.26.1 sources, oracle paths, and probe
artifacts. The first pass reported nine valid findings, all corrected:

1. scoped malformed-percent behavior by component and added raw-query/opaque
   Go goldens;
2. removed the false same-corpus claim for `iref` and limited every candidate
   claim to executed evidence;
3. added scheme lowercasing and empty-port removal;
4. stated IDNA-hostname/port-insensitive redirect policy and all six sensitive
   headers;
5. corrected the userinfo redirect observation to inherited userinfo clearing;
6. corrected the `iref` graph to 21 third-party crates plus probe root;
7. made target-check directories explicit; and
8. replaced global percent-preservation language with path, fragment, query,
   userinfo, and host-specific behavior and goldens; and
9. added and directly probed `oxiri`; it proved the strongest clean candidate,
   so the recommendation, graph, risks, and seam were revised accordingly.

The reviewer then reread all 384 lines of decision content at SHA-256
`fb3ae050078e7e7b06606d7b853dd72f85554272671559048ec3575e69a318a9`,
rechecked all nine corrections, and independently reran the Go precision probe,
the exact `oxiri` consumer run/test/Clippy/audit, upstream tests, four target
checks, metadata, and the `iref` advisory chain. Result: **CLEAN**, no actionable
finding; the reviewer remained read-only. A final exact-tip confirmation after
recording this review result also returned clean.

Affected package: `internal/dns` (`crates/ployz-internal-dns`).
