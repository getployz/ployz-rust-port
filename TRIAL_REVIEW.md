# Three-file port trial review

## Roles and independence

- Implementer: Codex, responsible for the original ports and adjacent tests; did not review the work.
- Reviewer A: Claude CLI `opus` alias at high effort in a fresh, no-persistence context; reviewed all three exact Go/Rust/test pairs and did not implement fixes.
- Reviewer B: Codex `gpt-5.6-sol` in a fresh context; its original pass used max effort before the user requested high effort for future agents; reviewed all three exact Go/Rust/test pairs and did not implement fixes.
- Fixer: separate Codex (this pass), neither the implementer nor either reviewer; received both reports, made the parity fixes, replayed the recurring classes, and did not commit.

Each trial file completed the required sequence:

| Go oracle | Rust port | Sequence |
| --- | --- | --- |
| `upstream/uncloud/internal/proxy/proxy.go` and `proxy_test.go` | `src/internal/proxy/proxy.rs` | implementer → Reviewer A → Reviewer B → separate fixer |
| `upstream/uncloud/internal/fs/fs.go` and `fs_test.go` | `src/internal/fs/fs.rs` | implementer → Reviewer A → Reviewer B → separate fixer |
| `upstream/uncloud/pkg/api/config.go` and `config_test.go` | `src/pkg/api/config.rs` | implementer → Reviewer A → Reviewer B → separate fixer |

## Finding disposition

### Independent report summaries

Reviewer A reported implicit half-close capability, loss of wrapped error text, broadened closed-error classification, weakened `EPIPE` identity testing, `ParseUint` grammar drift, missing JSON tag/byte semantics, unsafe process-environment testing, broad sorter visibility, added panic/unreachable behavior, trait-level accept cancellation requirements, fixed-width IDs, and UTF-8-only paths.

Reviewer B independently reported the absence of production TCP/Unix listener implementations, exposure of the internal tree, config symbols below the Go-package-equivalent module, extra `ECONNABORTED`/`ENOTCONN` classification, string-only wrapped errors, fixed `i64` for Go `int`, and UTF-8-only filesystem APIs.

Accepted and fixed:

- Proxy: added usable TCP and Unix listeners whose `close` cancels in-flight accept, drops the stored OS listener, permits TCP rebind, and unlinks/rebinds Unix sockets; made half-close support mandatory on every `Connection` implementation and added `UnixStream`; removed cancellation around the trait-level `accept`; removed the unreachable default-dialer network rejection and the `expect`-based invariant.
- Proxy: mapped Go's closed-network and closed-pipe sentinels explicitly and accepts only those or exact raw `EPIPE`/`ECONNRESET`. Synthetic `BrokenPipe`/`ConnectionReset`, `ECONNABORTED`, and `ENOTCONN` are distinct negative cases. The forwarding regression asserts `EPIPE`, listener failure retains a typed source, and dial timeout cancels its token and reports `context deadline exceeded` with a typed source.
- Filesystem: changed UID/GID results from fixed `i64` to platform-sized `isize`; changed path boundaries and home expansion to native `Path`/`OsStr` representations; restored the inner `chown path: errno` operation layer beneath the outer quoted context while preserving the `Errno` chain; serialized `HOME` mutation and restored it through a panic-safe guard.
- Config: rejected a leading `+` to match `strconv.ParseUint`, reproduced its visible parsing/invalid-syntax/value-out-of-range wording with a traversable local source, and checked parsed IDs against platform-sized `isize`. Added serde field-name/default/`omitempty` policy and Go-compatible base64 JSON for `[]byte`.
- Visibility: made the Rust `internal` tree crate-private, re-exported config symbols at `pkg::api`, and restricted `sort_config_mounts` to its package module.

The 42-line shared `src/error.rs` wrapper is the minimal direct-transcription exception needed to provide both Go-style concatenated `fmt.Errorf("...: %w")` display and Rust source-chain traversal at repeated wrapping sites. It contains no workflow, registry, policy, or extensibility mechanism and is not a porting framework.

No reported parity finding was rejected outright. Two implementation shortcuts were rejected while accepting the underlying findings: mapping every Rust `NotConnected` to Go `net.ErrClosed` would also accept `ENOTCONN`, so a dedicated sentinel is used; and retaining a cancellable trait-level `accept` would impose cancel-safety on every listener, so `run` instead relies on the source-compatible close-unblocks-accept contract. Broad cleanup, new frameworks, and unrelated features were out of scope and were not added.

## Replay across all three pairs

| Recurring class | Proxy replay | Filesystem replay | Config replay |
| --- | --- | --- | --- |
| Display plus source chain | accept/dial/copy wrappers preserve both | lookup/chown operation layers preserve both | Go-shaped parse and validation wrappers preserve both |
| Exact sentinels and identity assertions | explicit closed/pipe sentinels, raw errno matches, synthetic-kind negatives, direct `EPIPE` and listener-source assertions | no classifier; path-operation and underlying `Errno` chains asserted | no classifier; local `ParseUintError` and underlying `ParseIntError` chains asserted |
| Explicit optional capabilities/test doubles | all streams and doubles declare half-close | not applicable | not applicable |
| Primitive grammar and platform width | no public numeric parsing | Go `int` IDs map to `isize` | leading sign, overflow, and `isize` bound audited |
| Struct tags and byte encoding | not applicable | not applicable | PascalCase, omitted zero fields, decode defaults, base64 bytes tested |
| Package visibility | internal proxy remains crate-private | internal fs remains crate-private | API types re-exported; sorter package-private |
| Non-UTF-8 paths and environment safety | Unix listener accepts `Path` | path bytes preserved; `HOME` guarded | container path remains a Compose string, matching the Go field |
| Production I/O and shutdown | real TCP/Unix seams; close drops listeners, permits rebind, unlinks Unix path, and unblocks accept; half-close and dial deadline are explicit | not applicable | not applicable |
| Avoided added panic/unreachable behavior | default dial path no longer uses either | no added panic path | no added panic path |

Replay result: all recurring classes were checked against every pair; applicable gaps have regression coverage, and non-applicable cells do not expose the corresponding behavior.

## Verification

The fixer verification set is:

```text
cargo fmt --check
cargo check
cargo check --release
cargo test
cargo test --release
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
git diff --cached --check
git diff --exit-code -- upstream/uncloud
git diff --cached --exit-code -- upstream/uncloud
test "$(git rev-parse HEAD:upstream/uncloud)" = a1959e967bbde8577ed4a19d367e8ee4b1ecf2bd
```

All commands above passed in the final fixer verification.

## Human approval

**PENDING** — no human approval is claimed. Bulk translation must not begin until a human approves this trial.
