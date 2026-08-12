# Dependency decision: `go-text-template-compatible-caddy-rendering`

| Field | Value |
| --- | --- |
| Status | `approved` by explicit user authority on 2026-08-12, with the reviewed deviations below |
| Capability | Render user-authored Caddy configuration with the Go `text/template` language and the package's `upstreams` function |
| Selected dependency and exact version | `gotmpl = { version = "=0.6.1", default-features = false }` |
| Required features and configuration | No crate features; `std`, `serde`, `glob`, `html`, and `go-crosscheck` are intentionally absent from the production configuration. |
| License | `gotmpl 0.6.1` is MIT; all normal transitives in the probe use MIT, Apache-2.0, or Unicode-3.0 terms. |
| Research date | `2026-08-12` UTC |
| Exact research base | `886327c72d1db25ca56a6c76c65a2cdc6c4ee2b9` |
| Request | Existing `migration/DEPENDENCIES.tsv` registry row; no separate request record exists |
| Affected package | `internal/machine/caddyconfig` (`crates/ployz-internal-machine-caddyconfig`) |
| Accepted limitations | The documented diagnostic wording/location/context and parse-vs-execution phase differences, typed-map missing-index output edge, rare-language/reflection/formatting/resource-limit differences, and low adoption are accepted to unblock the port. |

## Decision

Approve `gotmpl 0.6.1` under the user's explicit 2026-08-12 dependency-unblock
authority. It is the strongest reviewed candidate because it is an
actively maintained pure-Rust port of Go `text/template`, supplies the
exercised syntax, exposes custom functions, and matches most principal
valid-output probes. The package must use the narrow package-private seam
described below and must not build a second generic Go template runtime.

The blocker is not cosmetic formatting around an otherwise equivalent error.
The frozen package places Go parse and execution errors into the generated
Caddyfile as user-visible comments, and the upstream tests assert those strings.
The provisional engine:

- produces materially different lexer/parser messages and locations;
- accepts undefined functions and variables during parse, then rejects them at
  execution, whereas Go rejects them during parse;
- omits Go's execution template name, source location, action excerpt, and
  `error calling FUNCTION` context;
- represents the Go context struct as a map, changing missing-field behavior;
  and
- exposes a public parser and positioned AST nodes, and `Template::lookup`
  exposes a non-empty named root such as this `Caddyfile` tree, but exposes no
  execution-error trace that identifies the active node, so package code
  cannot correlate a returned execution error with the failing action.

Exact arbitrary diagnostics therefore cannot be repaired by wrapping
`Template::parse` and `execute_to_string`. Repair would require a second
Go-compatible semantic parser/validator plus an execution-context interpreter
that finds the active action and reproduces Go's error taxonomy and wording.
That is a second template engine, not the narrow package-owned compatibility
seam allowed by the port workflow.

The only narrow viable provisional seam is the principal happy-path seam
described below: build the two-field dynamic value, register one `upstreams`
closure, parse, and execute. It preserves the common exercised documented
syntax and output, but even valid `{{index .Upstreams "missing"}}` diverges:
Go's typed map returns a nil `[]string`, rendered as `[]`, while the dynamic
candidate map returns untyped nil, rendered as `<no value>`. A package-owned
`index` override could special-case typed `Upstreams`, but to remain compatible
it would also have to preserve all list, string, and nested-index behavior and
errors; the public API exposes no way to delegate those cases to the displaced
built-in. The natural seam therefore does not clear either output or
observable-error hard gates.

Separately invoking the public `gotmpl::parse::Parser` does not make diagnostic
repair narrow. Package code would have to retain that second AST, reproduce
template calls and `if`/`with`/`range`/`break`/`continue` control flow, follow
pipelines and nested built-ins, and correlate every possible evaluator failure
with the active node and Go action spelling. That is execution tracing or a
second evaluator layered beside the crate's private executor, not a wrapper.

## Oracle and reachability

### Public product surface

The language is user-controlled and product-observable, not an implementation
detail:

- The product documentation says that `x-caddy` configs are processed as
  [Go templates](https://pkg.go.dev/text/template), links the full standard
  language, and documents `upstreams`, `.Name`, and `.Upstreams:
  [`publishing-services.md`](../../upstream/uncloud/website/docs/3-concepts/2-ingress/2-publishing-services.md).
- Compose accepts inline `x-caddy` text or loads a Caddyfile whose contents are
  stored in the service specification:
  [`pkg/client/compose/caddy.go`](../../upstream/uncloud/pkg/client/compose/caddy.go),
  [`pkg/client/compose/service.go`](../../upstream/uncloud/pkg/client/compose/service.go).
- CLI users can likewise supply a service or global Caddyfile:
  [`cmd/uc/service/run.go`](../../upstream/uncloud/cmd/uc/service/run.go),
  [`cmd/uc/caddy/deploy.go`](../../upstream/uncloud/cmd/uc/caddy/deploy.go).
- `CaddyfileGenerator.Generate` renders the newest user config for each
  service, renders the local `caddy` service's global config, validates each
  candidate, skips failures, and appends sorted `# Skipped invalid
  user-defined configs:` entries to the generated file:
  [`caddyfile.go`](../../upstream/uncloud/internal/machine/caddyconfig/caddyfile.go).
- The generated file, including those diagnostics, is stored and returned by
  the Caddy config API and printed by `uc caddy config`:
  [`service.go`](../../upstream/uncloud/internal/machine/caddyconfig/service.go),
  [`server.go`](../../upstream/uncloud/internal/machine/caddyconfig/server.go),
  [`cmd/uc/caddy/config.go`](../../upstream/uncloud/cmd/uc/caddy/config.go).

The repository itself uses the feature in
[`website/compose.yaml`](../../upstream/uncloud/website/compose.yaml) as
`{{upstreams 8000}}`. The product documentation also publishes examples for
all supported `upstreams` forms and for the advanced
`range $ip := index .Upstreams .Name` composition.

### Exact renderer entry point

[`renderCaddyfile`](../../upstream/uncloud/internal/machine/caddyconfig/caddyfile.go)
does exactly this:

1. creates a template named `Caddyfile`;
2. registers one package function named `upstreams` before parsing;
3. calls Go `text/template.Parse` on the complete user string;
4. on failure returns `parse config as Go template: ERROR`;
5. executes against the `templateContext` struct; and
6. on failure returns `execute template: ERROR`.

The input data is exactly the unexported struct in
[`template.go`](../../upstream/uncloud/internal/machine/caddyconfig/template.go):

```text
Name      string
Upstreams map[string][]string
```

`Name` is the service whose config is being rendered. `Upstreams` maps every
service name to the ordered IP strings of its healthy containers connected to
the Uncloud Docker network. The order is inherited from the generator's stable
record ordering: local machine first, then service name and container creation
time. Services with no qualifying container do not occur in the map.

### Exact user-authored template contract

Because the documented contract links Go `text/template` without narrowing
the grammar, the accepted Rust implementation must continue accepting the Go
1.26.1 text-template language that can operate on these values. The directly
exercised and documented subset is mandatory, not exhaustive:

| Surface | Exact required behavior |
| --- | --- |
| Delimiters and literal text | Actions use `{{` and `}}`; text outside actions is copied byte-for-byte. This package does not configure alternate delimiters. |
| Fields | `.Name` yields the current service name. `.Upstreams` yields the map. Chained field/map access follows Go evaluation rules. The top-level value is a struct, not a permissive map. |
| Built-in `index` | `index .Upstreams "service"` returns that service's `[]string`. Because this is a typed `map[string][]string`, a missing key returns nil `[]string`; printing it yields `[]`, while range produces no elements and truthiness is false. Nested indexing follows Go and out-of-range indexes are execution errors. |
| `range` | `{{range $ip := index .Upstreams "web"}}...{{end}}` sets `$ip` to each IP in slice order. One declared range variable receives the element; two receive index/key then element. Map ranges use Go's sorted order for ordered basic key types. `else`, nested branches, `break`, and `continue` remain part of the linked Go language. |
| Variables | `$` starts as the root context. `:=` declares and `=` assigns. Variable scope and restoration follow Go control-structure rules. The oracle tests exercise `$ip`, `$hostname`, `$upstreams`, `.Name` as an argument, and the product docs publish the `$ip` form. |
| Pipelines and controls | Go pipelines, parenthesized arguments, `if`, `with`, `range`, `else`, `template`, `block`, and the standard built-ins remain authored-language syntax. The package adds only `upstreams`; it does not remove Go's predefined built-ins. |
| Whitespace trimming | `{{-` trims immediately preceding Go whitespace only when the minus is followed by whitespace; `-}}` trims immediately following Go whitespace only when preceded by whitespace. Go whitespace here is space, horizontal tab, carriage return, and newline. The compound oracle case requires `{{- range ...}}` to remove the indentation before the generated `https://` upstream list. |
| Constants | Go boolean, string, raw-string, character, integer, floating, imaginary/complex, and nil syntax remains parseable according to Go's type and overflow rules. For `upstreams`, only the exact integer and string cases below are valid. |
| Output | Pipeline values use Go's textual representation; output before an execution failure may already have been written internally, but `renderCaddyfile` returns only an error and discards its buffer on failure. |

Primary language authority is the Go project's
[`text/template` documentation](https://pkg.go.dev/text/template) and exact
[Go 1.26.1 source](https://github.com/golang/go/tree/go1.26.1/src/text/template).
Package-level evidence is the compound oracle test in
[`caddyfile_test.go`](../../upstream/uncloud/internal/machine/caddyconfig/caddyfile_test.go),
which combines all principal forms and asserts the entire generated file.

### Exact `upstreams` function

[`upstreamsTemplateFn`](../../upstream/uncloud/internal/machine/caddyconfig/template.go)
is variadic and returns `(string, error)`. Its observable contract is:

| Template call | Lookup and output |
| --- | --- |
| `{{upstreams}}` | Current `.Name`, no port. |
| `{{upstreams 8080}}` | Current `.Name`, integer port `8080`. |
| `{{upstreams "api"}}` | Named service `api`, no port. |
| `{{upstreams "api" 9000}}` | Named service and integer port. |
| `{{upstreams .Name 8888}}` | Evaluated `.Name` string and integer port. |

Validation and formatting are exact:

- zero arguments selects the current service and port zero;
- one argument accepts Go dynamic type `int` or `string` only;
- two arguments require first dynamic type `string` and second dynamic type
  `int`;
- three or more arguments fail with
  `upstreams function: too many arguments; expected 0-2, got N`;
- the one-argument wrong-type error is
  `upstreams function: invalid argument type: TYPE` using Go `%T`;
- two-argument type errors are respectively
  `upstreams function: first argument must be service name (string)` and
  `upstreams function: second argument must be port (int)`;
- an absent service, a present empty slice, or no qualifying containers yields
  the empty string without error;
- IP order and duplicates are retained and entries are joined with one ASCII
  space;
- port `<= 0` yields each IP unchanged;
- port `> 0` applies Go `net.JoinHostPort`: IPv4 becomes
  `10.210.0.2:8080`, and IPv6 becomes `[fd00::1]:8080`;
- with no positive port, IPv6 remains bare (`fd00::1`).

### Observable errors

Both the phase and the complete string are observable. The package wraps the
Go error once, the generator wraps it again into the skipped-config summary,
and users can read the summary through the API/CLI. The frozen tests require,
among other cases:

```text
parse config as Go template: template: Caddyfile:3: unexpected "}" in operand
parse config as Go template: template: Caddyfile:2: unterminated quoted string
```

Execution errors use Go's `ExecError` form, including template name, line,
byte column, the action being executed, and call context. Exact Go 1.26.1
probe examples are:

```text
execute template: template: Caddyfile:1:2: executing "Caddyfile" at <upstreams true>: error calling upstreams: upstreams function: invalid argument type: bool
execute template: template: Caddyfile:1:2: executing "Caddyfile" at <upstreams "api" 80 90>: error calling upstreams: upstreams function: too many arguments; expected 0-2, got 3
execute template: template: Caddyfile:1:2: executing "Caddyfile" at <index (index .Upstreams "web") 9>: error calling index: index out of range: 9
```

Unknown registered functions and undeclared variables are parse errors in Go,
not deferred execution errors:

```text
parse config as Go template: template: Caddyfile:1: function "unknown" not defined
parse config as Go template: template: Caddyfile:1: undefined variable "$x"
```

Other syntax/type/bounds/field errors follow the same frozen Go 1.26.1
standard-library behavior. This record does not approve a hand-maintained list
that recognizes only the examples above; user-authored templates can exercise
the rest of the language.

### Internal generated base template

The same Go engine also renders the package-owned `caddyfileTemplate` using
`join`, `if or`, two ordered map ranges, variables, and left trim markers:
[`caddyfile.go`](../../upstream/uncloud/internal/machine/caddyconfig/caddyfile.go).
That source is not user-authored and Rust need not preserve it as an internal
template. Rust may generate the same Caddyfile output idiomatically. If it does
reuse the provisional engine, it must register a compatible `join` function
and preserve Go's sorted map-range output; that choice does not solve or reduce
the separate user-language/error blocker.

## Primary-source candidate evidence

### Strongest provisional candidate: `gotmpl 0.6.1`

- The official [0.6.1 documentation](https://docs.rs/gotmpl/0.6.1/gotmpl/)
  describes a Rust port of Go `text/template` with pipelines, control flow,
  variables, template composition, whitespace trimming, built-ins, custom
  functions, and Go-compatible output.
- The exact [0.6.1 manifest](https://github.com/phsym/gotmpl-rs/blob/v0.6.1/Cargo.toml)
  declares edition 2024, MSRV 1.89, MIT, and features `std`, `glob`, `serde`,
  `html`, and the test-only `go-crosscheck`. The selected provisional feature
  set is empty (`default-features = false`).
- The exact [MIT license](https://github.com/phsym/gotmpl-rs/blob/v0.6.1/LICENSE)
  is packaged in the crate. The published license hash observed by this
  research was
  `0cb8a86898ceaf86d95d6edaa496ed79bea66bbb336017c43ca043426a2bb0d6`.
- The exact release tag resolves to commit
  [`e675770a9617c57c8528d4ff2efba4c8f13fd9d2`](https://github.com/phsym/gotmpl-rs/tree/e675770a9617c57c8528d4ff2efba4c8f13fd9d2).
  Crates.io published 0.6.1 on 2026-07-13.
- The crate root has `#![forbid(unsafe_code)]` and denies panic-family lints:
  [`src/lib.rs`](https://github.com/phsym/gotmpl-rs/blob/v0.6.1/src/lib.rs).
  The truly minimal renderer enables no features. The optional `std` feature
  adds file/I/O helpers and catches panics in registered functions, but those
  are not needed for `parse`/`func`/`Value`/`execute_to_string`; the package's
  small `upstreams` function must itself remain panic-free.
- Its test suite contains a Go subprocess cross-check and 651 compatibility
  cases. With exact Go 1.26.1, all 267 unit, 651 compatibility, 98 Rust API,
  and 32 executed doctests passed in this research. The cross-check asserts
  byte output for success but only asserts that both sides fail for failure;
  it does not prove diagnostic identity.
- The official
  [`Differences from Go`](https://github.com/phsym/gotmpl-rs/blob/v0.6.1/README.md#differences-from-go)
  section explicitly records missing runtime source positions, single-error
  parsing, no reflection-backed struct fields/methods, no complex values,
  UTF-8/byte-string differences, and formatting differences.
- Its public dynamic [`Value`](https://github.com/phsym/gotmpl-rs/blob/v0.6.1/src/value.rs)
  has only an untyped `Nil`; map indexing returns that `Nil` for every missing
  key. The built-in
  [`index`](https://github.com/phsym/gotmpl-rs/blob/v0.6.1/src/funcs.rs)
  repeatedly invokes the same public `Value::index` operation. It cannot
  represent Go's typed nil `[]string` result for a missing `Upstreams` key.
- The source also imposes non-Go safety limits: parser nesting 100,
  execution nesting 200 rather than Go's 100,000, and a default total range
  budget of 10,000,000 iterations:
  [`parser.rs`](https://github.com/phsym/gotmpl-rs/blob/v0.6.1/src/parse/parser.rs),
  [`exec.rs`](https://github.com/phsym/gotmpl-rs/blob/v0.6.1/src/exec.rs).
- The crate publicly exports [`parse::Parser` and positioned AST
  nodes](https://github.com/phsym/gotmpl-rs/tree/v0.6.1/src/parse).
  `Template.tree` and its direct `root_tree` accessor are private, but parsing a
  non-empty named root also places it in the definition map, so public
  [`Template::lookup("Caddyfile")`](https://github.com/phsym/gotmpl-rs/blob/v0.6.1/src/lib.rs)
  can expose this use's root. That corrects an earlier overstatement but does
  not repair diagnostics: the private executor reduces failures to
  `TemplateError::Exec(String)` without returning the active node/action or a
  trace that correlates the error to the public tree.

Official ecosystem snapshots on 2026-08-12:

- the [crates.io API](https://crates.io/api/v1/crates/gotmpl) reported 374
  total downloads, 337 recent downloads, six releases since 2026-04-20, and
  non-yanked 0.6.1;
- the
  [reverse-dependency API](https://crates.io/api/v1/crates/gotmpl/reverse_dependencies?page=1&per_page=100)
  reported zero reverse dependencies;
- the [GitHub repository API](https://api.github.com/repos/phsym/gotmpl-rs)
  reported two stars, zero forks, an unarchived repository, a 2026-07-13
  release-source commit, and a 2026-07-17 Dependabot push.

This is current active maintenance and its declared MSRV passes. It is also a
four-month-old, minimally adopted crate. Low adoption is a serious durability
and selection risk under the workflow's preference for popular established
dependencies, but is not treated as a maintenance hard-gate failure. Its
behavior fit makes it stronger for this capability than popular engines with
the wrong language; approval would still need an explicit risk/behavior
decision after the parity blocker is addressed or waived.

### Rejected Go-FFI candidate: `gotpl 0.2.6`

The candidate search also inspected
[`gotpl 0.2.6`](https://crates.io/crates/gotpl/0.2.6), whose official manifest
describes full `text/template` and `html/template` support through Go FFI. It
cannot satisfy this capability:

- Its only public rendering interface is
  [`TemplateRenderer<T: Serialize>`](https://github.com/moyanj/gotpl/blob/v0.2.6/src/lib.rs).
  It JSON-serializes the context and exposes no function-registration API, so
  there is no natural way to register the required `upstreams` function.
- Its [Go bridge](https://github.com/moyanj/gotpl/blob/v0.2.6/src/go_ffi/ffi.go)
  unmarshals into `map[string]interface{}`, names the template `goTemplate`,
  and prefixes failures with `Failed to parse Text template:` or
  `Failed to execute Text template:`. JSON conversion removes the oracle's
  struct and typed `map[string][]string` semantics and decodes numbers as Go
  `float64`; the name and wrappers also differ.
- The exact release tag is commit
  [`031ca27b714bb6892b4b7c682b570c69c50f16ec`](https://github.com/moyanj/gotpl/tree/031ca27b714bb6892b4b7c682b570c69c50f16ec).
  Its exact MIT license was packaged with SHA-256
  `4dd6c0d4bc848bd9765b3c816601c7f19f7a996e436a6b9a5e9e746566cb371e`.
  Crates.io reported 2,458 total/119 recent downloads, one reverse dependency,
  and 478 downloads for exact 0.2.6 on 2026-08-12.
- The exact direct graph is `serde 1.0.225` with `derive`, `serde_json
  1.0.145`, and `bindgen 0.72.1` as both a normal and build dependency; the
  resolved lock contains 42 packages. The build script invokes `go build
  --buildmode=c-archive`, statically links the result, then runs bindgen over
  the generated C header. A consumer therefore needs a host Go/cgo/C toolchain
  and libclang at Cargo build time, and ordinary Rust target installation is
  insufficient for cross-compilation.
- Published source has an apparent memory-safety defect, so the runtime probe
  was deliberately not executed. It passes both input `CString`s with
  `into_raw` and never reclaims them. It converts output pointers allocated by
  Go's `C.CString` using Rust `CString::from_raw`, then `OwnedGoResult::drop`
  calls the Go `FreeResultString` (`C.free`) on the same pointers: an allocator
  mismatch followed by a second free. RustSec found no catalogued advisory in
  its 42-package lock, but an empty advisory result does not override the
  directly inspected unsafe ownership defect.

Thus using the actual Go standard library through this crate still fails
custom-function, value-model, error, security, and platform gates; it is not a
viable compatibility escape hatch.

### Rejected positioned-error fork: `gtmpl-ng 0.7.7`

Fresh review identified
[`gtmpl-ng 0.7.7`](https://crates.io/crates/gtmpl-ng/0.7.7), an actively
developed fork of old `gtmpl` whose official description specifically advertises
line-number fixes and whose source adds public structured parse/execution
errors. It is more relevant than the original `gtmpl`, but the adapted oracle
probe demonstrates that it does not displace `gotmpl`:

- The exact tag resolves to commit
  [`3098d1c3864592cc90ec4ba1878ce9a307efbcd1`](https://github.com/firstdorsal/gtmpl-rust/tree/3098d1c3864592cc90ec4ba1878ce9a307efbcd1)
  and crates.io published it on 2026-04-09. Its manifest marks it actively
  developed but declares no MSRV. The exact MIT license has SHA-256
  `3a4008a31d3f313ce355a9b806ea3c5d30b010f346f01d537eb7d82e85cefdd8`.
  Official snapshots reported 619 total/303 recent downloads and zero reverse
  dependencies. The repository had one star and zero forks. Four releases
  since 2026-01-06 and the active-development badge support a maintenance
  pass, while this remains a young, minimally adopted fork.
- Its minimal configuration is
  `gtmpl-ng = { version = "=0.7.7", default-features = false }`; none of the
  Helm, Mows, or dynamic-template features is required. The resolved normal
  graph is `anyhow 1.0.104`, `gtmpl_value 0.5.1`, `lazy_static 1.5.0`,
  `percent-encoding 2.3.2`, and `thiserror 1.0.69`, plus
  `thiserror-impl 1.0.69`, `proc-macro2 1.0.107`, `quote 1.0.47`, `syn
  2.0.119`, and `unicode-ident 1.0.24`. All are MIT or MIT/Apache-2.0, with
  Unicode-3.0 additionally applying to `unicode-ident`.
- Its [`Template::add_func`](https://github.com/firstdorsal/gtmpl-rust/blob/v0.7.7/src/template.rs)
  accepts `gtmpl_value::Func`, which is only a plain
  `fn(&[Value]) -> Result<Value, FuncError>` pointer. It cannot capture the
  per-render `.Name` and `.Upstreams` needed by zero- and one-argument
  `upstreams`. Thread-local or global mutable render state would introduce
  concurrency, reentrancy, and cleanup hazards and is not the dependency's
  natural narrow API.
- It does reject undefined functions and variables during parse and its
  [`StructuredError`](https://github.com/firstdorsal/gtmpl-rust/blob/v0.7.7/src/error.rs)
  carries name, line, column, and node length. Its display is nevertheless
  `template: NAME:LINE:COL:LENGTH:MESSAGE`, not Go's parse/`ExecError` text;
  execution failures still omit the action excerpt and `error calling ...`
  context.
- Its value model uses `HashMap`, debug-prints values such as
  `[String("10.0.0.2"), ...]`, returns `<no value>` for a missing map index,
  and iterates maps in randomized rather than Go sorted-key order. Two
  consecutive executions produced different `.Upstreams` displays and range
  order. It also rejects Go variable reassignment with `=`, and treats Go
  `break` and `continue` as undefined functions at parse time. Its executor
  runs a `range` `else` body even after iterating a non-empty collection, and
  it omits Go's predefined `slice` built-in. Its own README also continues to
  disclaim complex numbers, `html`/`js`, and stable `printf` parity.
- With debug overflow checks, the frozen malformed-action inputs trigger an
  arithmetic-underflow panic in the crate's spawned lexer thread, emit the
  panic hook to stderr, and surface `receiving on a closed channel` instead of
  either Go parse error. In a release build the same subtraction wraps: one
  diagnostic reports column `18446744073709551608`. Direct inspection found no
  `unsafe` in the `gtmpl-ng` or `gtmpl_value` engine/value crates, and RustSec
  found no advisory in the exact resolved consumer lock; some general-purpose
  transitive crates do use unsafe internally. User-controlled malformed input
  reaching a build-mode-dependent panic/wrap path remains an additional
  robustness failure regardless.

The synchronized 40-case differential run produced only 20 exact rows and 20
differences: all 12 oracle-error rows differed, along with valid map display,
existing and missing map `index`, sorted map range, non-empty `range` with
`else`, variable reassignment, `break`, and `continue`. The same five Rust
targets compiled under Rust 1.96, but behavior, natural integration, and
robustness gates fail first.

Its exact graph has an effective MSRV of Rust 1.71: it failed under 1.68.2
because `quote` requires 1.71 and passed under 1.71.1 and the workspace's 1.96.
The exact no-feature upstream suite passed 66 unit, six integration, and 22
doctests. Upstream CI runs only Ubuntu, so this research's five compile targets
remain the platform evidence rather than a claim of upstream cross-platform CI.

### Rejected passive republish: `gtmpl-moyan 0.7.1`

[`gtmpl-moyan 0.7.1`](https://crates.io/crates/gtmpl-moyan/0.7.1) is a
2025-09-26 republish of the passive old gtmpl line with small patches. Its
exact minimal line has no features; its normal graph, effective Rust 1.71
minimum, five successful target checks, permissive MIT/transitive licenses,
and clean fresh-consumer RustSec result are the same as `gtmpl-ng`'s core
graph. Crates.io reported 668 total/90 recent downloads and one reverse
dependency. Its exact manifest remains `passively-maintained` and points to
the original inactive repository. The packaged VCS metadata names commit
`49418954786c6c0e9f710dca1cf5cc320135b0e1`, but that object/tag is absent
from the declared public repository and no matching public publisher
repository was found; the release therefore lacks verifiable public source
provenance beyond the published crate.

The adapted 40-case probe also yielded only 20 exact/20 different rows, with
the same function-pointer, value-display, typed-index, range-order/`else`,
assignment, and `break`/`continue` failures and without `gtmpl-ng`'s structured
execution positions. Its added `slice` implementation directly indexes Rust
strings/arrays: user-authored `{{slice "x" 9}}` panics the main process at
`funcs.rs:571` with exit 101. It is strictly weaker than both `gtmpl-ng` and
the provisional `gotmpl`.

## Hard-gate results

| Gate | Requirement | Primary evidence | Result |
| --- | --- | --- | --- |
| Observable behavior | Preserve user-authored Go syntax, byte output, `upstreams`, and observable parse/execution errors | Principal documented valid cases pass, but direct indexing of a missing typed upstream key already differs (`[]` versus `<no value>`). Error phase, location, context, and wording also fail as detailed below. The public API supplies no execution-error-to-node/action correlation with which package code could recover Go context. | **`fail`** |
| License | Permissive license compatible with the port | `gotmpl` is MIT. Normal graph licenses are MIT, Apache-2.0, or Unicode-3.0 terms. | `pass` for provisional graph |
| Security | Safely parse user-controlled template text without native code, hidden I/O, arbitrary code exposure, or known advisory | Crate forbids unsafe. Only registered/built-in functions are callable; the provisional package would register one small panic-free `upstreams` closure. RustSec found no advisory in the exact no-feature graph. Resource limits improve denial-of-service resistance but observably differ from Go and are unapproved. | `pass` for known dependency security; does not cure behavior failure |
| Platforms and targets | Linux amd64/arm64 daemon; compile on project macOS and Windows target families without native system dependency | The exact consumer graph checked on `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-gnu`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`. It is pure Rust. Upstream CI tests only Ubuntu; macOS/Windows entries are commented out. | `pass` for compilation; non-Linux native runtime not claimed |
| Maintenance and Rust version | Maintained and Rust 1.96 compatible; adoption informs candidate ranking and durability risk | Exact release MSRV 1.89 and Rust 1.96 builds pass; release activity is current. Official metrics show only 374 downloads, zero reverse dependencies, and two stars. | `pass` for maintenance/MSRV; serious low-adoption risk |
| Architectural constraints | Use the dependency's natural API and only a narrow package seam; no local Go-template reimplementation | Happy-path seam is narrow. Exact diagnostic repair requires a second parser/semantic validator/execution-context interpreter because the engine discards required information and changes phase. | **`fail`** |

Overall result: **blocked**. Passing license, current-advisory, and platform
checks do not override the behavior and architecture failures.

## Differential probe results

The synchronized runnable probe used one identical ordered 40-case matrix and
identical `gateway`/`web`/`empty` data on both sides. The Go driver used the
exact frozen struct shape and copied the oracle's `upstreamsTemplateFn` logic
into a standalone Go 1.26.1 renderer. The Rust driver used exact `gotmpl 0.6.1`,
a `Value::Map` for `Name`/`Upstreams`, and one custom closure. Both emitted
three tab-separated fields (`case`, `phase`, quoted output/error). Case-name
diffing proved the same 40 cases in the same order. Full-row comparison found
27 byte-identical rows and 13 differences: the one valid typed-map-index
output difference and all 12 cases that produce an oracle error. The following
are representative results; successful rows marked pass were byte-equal:

| Case | Go 1.26.1 | `gotmpl 0.6.1` | Result |
| --- | --- | --- | --- |
| `.Name`; existing `index .Upstreams "web"` | `gateway`; `[10.0.0.2 2001:db8::2]` | same | pass |
| `range $ip := index ...` plus trim | `Ahttps://10.0.0.2;https://2001:db8::2; Z` | same | pass |
| `upstreams` zero arguments | `10.0.0.1 2001:db8::1` | same | pass |
| one integer port | `10.0.0.1:8080 [2001:db8::1]:8080` | same | pass |
| one string service | `10.0.0.2 2001:db8::2` | same | pass |
| string plus integer port | `10.0.0.2:9000 [2001:db8::2]:9000` | same | pass |
| `.Name` plus port | `10.0.0.1:7000 [2001:db8::1]:7000` | same | pass |
| missing service | empty replacement | same | pass |
| `index .Upstreams "missing"` | `[]` (typed nil `[]string`) | `<no value>` (untyped `Value::Nil`) | **fail** |
| zero/negative port | bare IPv4/IPv6 | same | pass |
| variable declaration and reassignment | `gateway` | same | pass |
| one boolean argument | full Go `ExecError` with action and custom message | `execute template: execution error: upstreams function: invalid argument type: bool` | **fail** |
| one float argument | full Go `ExecError` and `float64` | positionless Rust `Exec` | **fail** |
| non-string first of two | full Go call context | positionless Rust `Exec` | **fail** |
| non-int second of two | full Go call context | positionless Rust `Exec` | **fail** |
| three arguments | full Go call context | positionless Rust `Exec` | **fail** |
| malformed `{{upstreams` across lines | line 3, `unexpected "}" in operand` | line 3 column 1, `unexpected character: '}'` | **fail** |
| unterminated string on line 2 | line 2, `unterminated quoted string` | line 3 column 2, `unterminated string` | **fail** |
| unknown function | parse error | parse succeeds; execution `undefined function` | **fail** |
| undeclared variable | parse error | parse succeeds; execution `undefined variable` | **fail** |
| nested index out of range | full Go `error calling index` action context | `index out of range: 9` only | **fail** |
| `.Missing` | Go struct-field execution error | successful `<no value>` under Rust map default | **fail** |

Using `MissingKey::Error` can make the final case return an error, but its text
is `map has no entry for key: Missing`, still lacks position/action context,
and changes Go's ordinary missing-map-key semantics. It is not a repair.

## Candidate comparison and rejected alternatives

Official crates.io counts below are 2026-08-12 snapshots.

| Candidate | Behavior and natural fit | Maintenance and adoption | Build/license | Decision |
| --- | --- | --- | --- | --- |
| [`gotmpl 0.6.1`](https://crates.io/crates/gotmpl/0.6.1) | Purpose-built modern Go `text/template` port; most principal Caddy syntax/output probes pass and its custom closure API is natural, but missing typed-map indexing already changes valid output. Explicit runtime-diagnostic, parse, reflection, value, formatting, and limit deviations fail required parity. | Current July 2026 release; 374 total/337 recent downloads; zero reverse dependencies; two GitHub stars. | MIT; MSRV 1.89; pure Rust; exact minimal configuration has no features and a modest graph. | **Strongest provisional candidate, blocked.** |
| [`gotpl 0.2.6`](https://crates.io/crates/gotpl/0.2.6) | Calls actual Go templates, but exposes no custom-function registration; JSON changes struct, typed-map, and number semantics; template/error names and wrappers differ. Published FFI ownership appears to leak inputs and allocator-mismatch/double-free outputs. | October 2025 release; 2,458 total/119 recent downloads; one reverse dependency. | MIT; no declared MSRV; 42-package graph plus required Go/cgo/C/libclang and build-time bindgen. | Rejected at custom-function, behavior, security, and platform gates. |
| [`gtmpl-ng 0.7.7`](https://crates.io/crates/gtmpl-ng/0.7.7) | Active gtmpl fork with parse-time function/variable validation and structured positioned errors. However its error shape is still not Go's; it lacks `=` reassignment and `break`/`continue`, changes list/map rendering and missing index, randomizes map order, executes `range`'s `else` after non-empty ranges, and malformed oracle inputs hit a debug-panic/release-wrap lexer defect. A plain function-pointer API cannot capture each render's context for `upstreams`. | April 2026 release; 619 total/303 recent downloads; zero reverse dependencies; manifest says actively developed. | MIT; no declared MSRV; no features needed; pure-Rust 12-package consumer graph; all five target checks pass. | Rejected at syntax/output/error, natural-API, and robustness gates; 20/40 probe rows differed. |
| [`gtmpl 0.7.1`](https://crates.io/crates/gtmpl/0.7.1) | Older Go-template implementation with custom functions and broad syntax, but its [README](https://github.com/fiji-flo/gtmpl-rust/blob/0.7.1/README.md) calls it imperfect/work-in-progress, omits `html`/`js`, and says `printf` is not stable. No evidence makes its diagnostics more compatible. | 851,755 total/143,633 recent downloads and ten reverse dependencies, but release was 2021-08-06; repository last source push 2022-03-30; manifest labels it passively maintained. | MIT; older multi-crate value/derive design; no declared MSRV. | Rejected by maintenance and behavior-proof gates despite historical popularity. |
| [`gtmpl-moyan 0.7.1`](https://crates.io/crates/gtmpl-moyan/0.7.1) | September 2025 republish of the old gtmpl code line with small patches, including a `slice` built-in that directly byte-slices strings/arrays and panics on user bounds. It retains the same function-pointer/value/parser architecture, lacks positioned execution errors, and matches only 20/40 probe rows. | One release; 668 total/90 recent downloads; one reverse dependency; exact manifest says passively maintained and points to the inactive original repository; packaged commit is absent there. | MIT; no declared MSRV (resolved graph needs 1.71); no features; same permissive core graph/five target passes. | Rejected as redundant, passive, provenance-poor, behavior-inferior, and security-failing. |
| [`go-template 0.0.3`](https://crates.io/crates/go-template/0.0.3) | An old gtmpl-family fork/copy using `gtmpl_value 0.5`; it offers no documented language or diagnostic advantage over gtmpl for this contract. | Published 2022-09-07; 7,222 total/136 recent downloads; zero reverse dependencies; its exact manifest labels maintenance `passively-maintained`. | MIT; edition 2018; no declared MSRV; older anyhow/lazy_static/percent-encoding/thiserror graph. | Rejected as passive, stale, and behavior-unproven; included for search completeness. |
| [`lithos-gotmpl-engine 0.1.0`](https://crates.io/crates/lithos-gotmpl-engine/0.1.0) plus `lithos-gotmpl-core 0.1.0` | Official [README](https://github.com/hans-d/lithos-gotmpl-rs) says it intentionally supports only a subset and specifically lacks `else if`, `define`/`template`, and `break`. | First release 2025-11-01; engine 1,394 downloads; two reverse dependencies; zero repository stars. Source pushed 2026-08-05 but no newer published release. | MIT OR Apache-2.0; serde/serde_json/thiserror graph. | Rejected: explicit syntax hard-gate failure and no adoption advantage. |
| [`MiniJinja 2.23.0`](https://crates.io/crates/minijinja/2.23.0) | Mature runtime engine, but its [official README](https://github.com/mitsuhiko/minijinja/tree/2.23.0) targets Jinja2 syntax/behavior. Custom delimiters cannot turn Jinja control expressions, variables, functions, or errors into Go's language. | 29.1M downloads; active 2026-08-06 release; 2,729 GitHub stars. | Apache-2.0; MSRV 1.70; configurable feature graph. | Rejected at syntax hard gate. A translator would be a second parser and would alter errors. |
| [`Handlebars 6.4.4`](https://crates.io/crates/handlebars/6.4.4) | Mature dynamic engine whose [official README](https://github.com/sunng87/handlebars-rust/tree/v6.4.4) implements Handlebars syntax, helpers, and errors rather than Go templates. | 86.8M downloads; current 2026-08-12 release; 1,479 GitHub stars. | MIT; MSRV 1.85. | Rejected at syntax hard gate. |
| [`Upon 0.11.0`](https://crates.io/crates/upon/0.11.0) | Compact runtime engine, but its [official syntax](https://github.com/rossmacarthur/upon/blob/0.11.0/SYNTAX.md) is inspired by Liquid/Jinja and uses different controls, access, and functions. | 427,012 downloads; active 2026-07-25 release; 65 GitHub stars. | MIT OR Apache-2.0; MSRV 1.66. | Rejected at syntax hard gate. |
| Hand-written parser/evaluator or Go helper subprocess | Could be designed toward exact language/error identity | No dependency adoption; recreates the oracle engine or keeps Go at runtime | Material local language runtime or external toolchain/process lifecycle | Rejected by the dependency-natural-design/narrow-seam requirement. |

Popular Jinja/Handlebars/Liquid-family engines are not stronger candidates for
this capability: popularity cannot compensate for silently changing a
documented user-authored language.

## Approved integration seam

The explicit 2026-08-12 authority selects this narrow candidate-shaped
implementation:

```toml
gotmpl = { version = "=0.6.1", default-features = false }
```

Use the dependency's natural model:

1. Construct a `gotmpl::Value::Map` with exact keys `Name` and `Upstreams`.
   Build `Upstreams` as a sorted-key map whose values are ordered string lists.
   Do not enable Serde merely to model these two fields.
2. Register one `upstreams` closure before parse. Pattern-match exact
   `gotmpl::Value::Int` and `Value::String` variants and preserve the oracle's
   0/1/2/many argument policy and messages inside the closure.
   This alone cannot reproduce the typed missing-map value returned by
   `index .Upstreams`. Replacing `index` would require package code to preserve
   the built-in's list, string, and arbitrarily nested index semantics and
   errors as well, because the public function API provides no delegation to
   the shadowed built-in. That expansion is not part of this narrow seam.
3. For positive ports, parse/format addresses with Rust standard IP address
   types or an equivalently narrow host/port formatter so IPv6 receives one
   bracket pair. Keep non-positive-port IP text unchanged.
4. Call `Template::new("Caddyfile").func(...).parse(source)` and
   `execute_to_string(&context)`.
5. Keep this renderer private to the caddyconfig package. Do not expose a
   Go-shaped generic template wrapper or imitate Go's `template.Template` API.
6. Render the package-owned base Caddyfile directly in Rust unless using the
   engine materially simplifies exact output; it is not part of the public
   authored-language seam.

This seam is permission to consume only the exact crate/configuration above.
It preserves most principal exercised valid output but has the accepted
deviations below.

## Accepted limitations

The explicit 2026-08-12 authority accepts these reviewed limitations:

- all parse-message/location differences demonstrated by the probe;
- undefined functions/variables moving from parse to execution;
- missing execution template name, line, column, action, and call context;
- struct-field misses becoming map misses or `<no value>`;
- missing `Upstreams` keys indexed directly rendering `<no value>` instead of
  Go's `[]` for the typed nil `[]string` map value;
- first-error-only parsing;
- parser depth 100, executor depth 200, and 10,000,000 total range-iteration
  budget instead of Go's behavior;
- no complex value representation and the imaginary-literal divergence;
- UTF-8-only strings versus Go byte strings, including octal escape and
  mid-codepoint string-slice behavior;
- documented `%#v`, `%#U`, NaN, reflection/method, typed-nil, channel, and
  iterator differences; and
- the maintenance/adoption risk of a new crate with no registered dependents.

The package must characterize the reachable differences and keep the seam
private; it must not expand into a second generic template runtime.

## Verification commands and evidence

### Frozen package oracle

Executed at exact repository base with the reproduced Go toolchain:

```sh
cd upstream/uncloud
GOTOOLCHAIN=local /opt/go1.26.1/bin/go test -count=1 \
  ./internal/machine/caddyconfig
```

Result: `ok`.

### Differential probe

The temporary probe was outside the repository at
`/tmp/ployz-gotmpl-probe`. `probe.go` and `src/main.rs` contained the same 40
case names, order, source strings, context names, and IP data; each emitted the
same three-field format. Commands:

```sh
cd /tmp/ployz-gotmpl-probe
GOTOOLCHAIN=local /opt/go1.26.1/bin/go run probe.go > go126.out
cargo run --quiet > rust.out
cut -f1 go126.out > go.cases
cut -f1 rust.out > rust.cases
diff -u go.cases rust.cases
diff -u go126.out rust.out
awk -F '\t' \
  'NR==FNR { go[$1]=$0; ng++; next }
   { nr++; if (go[$1] == $0) same++; else different++ }
   END { printf "go=%d rust=%d exact=%d different=%d\n", ng, nr, same, different }' \
  go126.out rust.out
```

The case-name diff was empty: 40 Go rows and 40 Rust rows in identical order.
An exact keyed full-row comparison reported `exact=27 different=13`. The 27
identical rows include `.Name`, `.Upstreams`, existing-key `index`, range and
map ordering, variables, assignment, `if`/`with`, whitespace and comment trim,
all successful zero/one/two-argument `upstreams` forms, absent service,
zero/negative port, IPv4/IPv6, `break`/`continue`, `define`/`template`, and
`block`. The 13 differing rows are the valid missing typed-map index plus all
12 oracle-error cases (`pipeline` type error; five direct `upstreams` argument
errors; missing field/function/variable; out-of-bounds index; malformed
operand; unterminated string). The complete `comparison.out` preserves every
pair shown by `diff`. The probe is reproducible evidence, not a repository
artifact; this decision owns no `migration/probes` or `research/` file.

The same Go output and case matrix were then compared with a separate
`gtmpl-ng 0.7.7` driver at `/tmp/ployz-gtmpl-ng-probe`. Because its public
custom-function type cannot capture state, the driver hard-coded only the
identical fixture data inside its test function and explicitly did not treat
that as an integration seam:

```sh
cd /tmp/ployz-gtmpl-ng-probe
cargo run --quiet > gtmpl-ng.out 2> gtmpl-ng.stderr
cut -f1 gtmpl-ng.out > rust.cases
diff -u go.cases rust.cases
awk -F '\t' \
  'NR==FNR { go[$1]=$0; ng++; next }
   { nr++; if (go[$1] == $0) same++; else different++ }
   END { printf "go=%d rust=%d exact=%d different=%d\n", ng, nr, same, different }' \
  go126.out gtmpl-ng.out
cargo run --quiet > run2.out 2> run2.stderr
diff -u gtmpl-ng.out run2.out
```

The case-name diff was empty and comparison reported
`go=40 rust=40 exact=20 different=20`. The second-run diff changed printed map
and ranged-map order, confirming nondeterminism. Both malformed oracle inputs
emitted `attempt to subtract with overflow` panics from `src/lexer.rs` to the
captured debug-build stderr and returned a closed-channel parse error. A
separate `cargo run --release` emitted no panic hook but returned different
wrong errors, including the wrapped column `18446744073709551608`.

### Exact candidate upstream cross-check

The exact release tag was cloned and tested with its Go cross-check feature:

```sh
git clone --depth 1 --branch v0.6.1 \
  https://github.com/phsym/gotmpl-rs.git /tmp/gotmpl-rs-v0.6.1
cd /tmp/gotmpl-rs-v0.6.1
PATH=/opt/go1.26.1/bin:$PATH GOTOOLCHAIN=local \
  cargo test --no-default-features --features std,go-crosscheck
```

Result: 267 unit tests, 651 Go-compatibility tests, 98 Rust API tests, and 32
executed doctests passed; three doctests were intentionally ignored. `std` in
this command is required by the upstream Go-subprocess cross-check harness, not
by the no-feature production renderer. Rebuilding and rerunning the local
renderer with no features produced byte-identical output to its earlier
`std`-feature probe.

### Platform and graph checks

With Rust/Cargo 1.96.0 and the exact provisional manifest:

```sh
cd /tmp/ployz-gotmpl-probe
cargo tree -e normal
cargo check --locked --target x86_64-unknown-linux-gnu
cargo check --locked --target aarch64-unknown-linux-gnu
cargo check --locked --target x86_64-pc-windows-gnu
cargo check --locked --target x86_64-apple-darwin
cargo check --locked --target aarch64-apple-darwin
```

All checks passed. The exact normal external graph was:

```text
gotmpl 0.6.1
├── smol_str 0.3.6
└── thiserror 2.0.20
    └── thiserror-impl 2.0.20
        ├── proc-macro2 1.0.107
        │   └── unicode-ident 1.0.24
        ├── quote 1.0.47
        └── syn 3.0.3
```

`gotmpl` is MIT; all listed transitive crates are MIT OR Apache-2.0, with
`unicode-ident` additionally carrying Unicode-3.0 terms. The graph has no
native library, build script, runtime, network client, storage, subprocess, or
FFI dependency.

The rejected `gtmpl-ng` minimal consumer was checked with the corresponding
exact locked commands:

```sh
cd /tmp/ployz-gtmpl-ng-probe
cargo tree -e normal
cargo audit --file Cargo.lock --db /home/codex/.cargo/advisory-db
cargo check --locked --target x86_64-unknown-linux-gnu
cargo check --locked --target aarch64-unknown-linux-gnu
cargo check --locked --target x86_64-pc-windows-gnu
cargo check --locked --target x86_64-apple-darwin
cargo check --locked --target aarch64-apple-darwin
```

All target checks passed under Rust 1.96 and the exact resolved 12-package
consumer lock had no RustSec advisory. The release's own packaged broad
lock still pins advisory-flagged `anyhow 1.0.102` and optional `rand 0.9.2`;
the fresh minimal consumer lock excludes `rand` and resolved fixed `anyhow
1.0.104`. Neither result mitigates the directly reproduced lexer-thread panic
on malformed user input.

### Security check

```sh
cd /tmp/ployz-gotmpl-probe
cargo audit
```

Result: no vulnerabilities reported in the exact lock. The advisory database
was commit `69f93e1d081d8b6fbee010e48f0b5e0d13661415`, dated 2026-08-12, with
1,216 loaded advisories. This is a time-bounded advisory result, not a claim
that future versions or undiscovered issues are safe.

### Rejected FFI candidate inspection

Exact published `gotpl 0.2.6` source was fetched through Cargo. The following
read-only commands resolved its build/normal graph, checked its packaged
license, checked the same RustSec database, and located the ownership calls:

```sh
cargo info gotpl@0.2.6
cargo tree --manifest-path "$GOTPL_SOURCE/Cargo.toml" -e normal,build
sha256sum "$GOTPL_SOURCE/LICENSE"
cargo audit --file "$GOTPL_SOURCE/Cargo.lock" \
  --db /home/codex/.cargo/advisory-db
rg -n 'into_raw|from_raw|FreeResultString|C.CString' "$GOTPL_SOURCE/src"
```

Here `GOTPL_SOURCE` denoted Cargo's exact unpacked
`registry/src/.../gotpl-0.2.6` directory, not repository state. The resolved
graph had 42 packages and the direct versions listed in the candidate evidence;
RustSec reported no catalogued advisory. Source inspection established the
input leaks and output allocator-mismatch/double-free path described above.
No render was executed because invoking code with that ownership path would
not be a responsible compatibility test. The build script itself establishes
the Go/cgo/archive/bindgen/libclang cross-target burden, so it was rejected
without treating a host-only build as platform proof.

## Resolved unblock condition

The research-time dependency gate required one of these conditions:

1. a maintained Rust crate exposes Go-compatible parse and execution
   diagnostics, including phase and source/action context, and passes a fresh
   differential corpus; or
2. explicit product authority accepts a material change to template errors
   and the other enumerated deviations, plus the maintenance/adoption risk of
   the provisional crate.

The user selected option 2 on 2026-08-12. The accepted limitations and exact
dependency are recorded above and in `migration/AUTHORITIES.tsv`.

## Review

First read-only adversarial review, task `/root/adversarial_review_1`: **not
clean**. It reported four findings, all addressed before the fresh re-review:

1. Candidate coverage omitted `gotpl 0.2.6` and `go-template 0.0.3`. The record
   now includes both. It gives `gotpl`'s exact FFI/build graph, absent custom
   function API, JSON and diagnostic mismatches, license/advisory result,
   cross-platform burden, and apparent input-leak/output-double-free defects;
   it also identifies and rejects the passive old gtmpl-family fork.
2. The temporary Go and Rust probe drivers had drifted and old saved output was
   not comparable. Both drivers were rebuilt around the same 40 cases, order,
   data, and three-field output format; fresh files were generated and exact
   case/full-row comparisons are recorded above.
3. The record overstated the absence of AST positions. It now acknowledges the
   public parser and positioned nodes. A later review correctly identified
   that `Template::lookup("Caddyfile")` also exposes a non-empty named root;
   the precise blocker is the missing execution-error-to-active-node/action
   trace, including why independently replaying the AST becomes an evaluator.
4. Maintenance and adoption were conflated. Current maintenance and MSRV now
   pass; minimal adoption remains a serious ranking/durability risk, while the
   blocked verdict rests on observable behavior and narrow-seam architecture.

Second fresh read-only adversarial review, task
`/root/adversarial_review_2`: **not clean**. It independently verified the
first review's fixes and reported three further findings, all now addressed:

1. The search omitted `gtmpl-ng 0.7.7` and `gtmpl-moyan 0.7.1`. Both are now
   dispositioned. The active positioned-error fork received its own 40-case
   differential driver, exact no-feature graph, license/advisory/MSRV/target
   checks, maintenance/adoption evidence, and source review. It is weaker than
   `gotmpl`: 20 rows differ, its plain function pointer cannot capture render
   context, and it has additional syntax/order/range/lexer defects.
2. `gotmpl::Template::lookup("Caddyfile")` can expose this non-empty named
   root. All contrary claims were removed; the blocker is precisely the lack
   of execution-error-to-active-node/action correlation from the private
   evaluator.
3. `gotmpl`'s required renderer API needs no feature. The provisional line and
   all graph/security/platform claims now use `default-features = false` with
   an empty feature set; `std` appears only in the upstream cross-check command
   and is explicitly test-only there.

Third fresh read-only adversarial re-review of the twice-corrected record:
**not clean**. Task `/root/adversarial_review_3` verified every behavioral,
probe, candidate, API, graph, license, audit, platform, scope, and blocked-
verdict claim, but found one medium wording error: the record claimed the whole
minimal `gtmpl-ng` graph contained no unsafe code. That was narrowed to the
direct engine/value crates; transitive crates may use unsafe internally.

Fourth fresh read-only adversarial re-review after that correction:
**clean**. Task `/root/adversarial_review_4` found no actionable factual,
scope, security, platform, candidate-coverage, or reproducibility issue. It
independently reran both synchronized matrices (`gotmpl`: 40 cases, 27 exact,
13 different; `gtmpl-ng`: 40 cases, 20 exact, 20 different), verified the
no-feature manifests and exact normal graphs, checked the oracle wrappers and
contract, confirmed the narrowed direct-crate unsafe wording, and found only
the owned untracked record at the unchanged base. It affirmed that typed-map
valid-output divergence plus material parse/execution phase/context/error
incompatibilities have no narrow natural-API repair and support the blocked
verdict.

This template engine is a synchronous, side-effect-free parser/evaluator and
does not fall into the workflow's critical networking/storage/cryptography/
runtime/service list. The user's assignment nevertheless explicitly requires
an adversarial research review, so that review remains mandatory.

Affected package and state at research base:

- `internal/machine/caddyconfig` / `crates/ployz-internal-machine-caddyconfig`;
- package state `dependency-blocked`;
- blockers `caddy-admin-http-over-unix-socket` and this capability;
- caddyconfig owner task existed in migration state but had no package result
  or crate changes available in its worktree at research time.
