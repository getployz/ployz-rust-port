# Package packet: `internal/gitutil`

## Assignment

| Field | Value |
| --- | --- |
| Go package | `upstream/uncloud/internal/gitutil` |
| Migration crate | `ployz-internal-gitutil` |
| Owned path | `crates/ployz-internal-gitutil/**` |
| Base commit | `565209151fc54d459bb6ff8de6f4c3793faedc7e` |
| Wave | `0` |
| State | `catalogued` |

The implementor owns only the path above. The integrator owns root workspace
files. The controller owns this packet and registries. This packet is contract
complete, but the package cannot become `ready` until the research-required
capability below has an approved dependency decision.

## Oracle inventory

### Package files

| Category | Files and purpose |
| --- | --- |
| Direct Go source | `upstream/uncloud/internal/gitutil/state.go` — repository inspection, command execution, state model, and SHA shortening |
| Upstream tests | `upstream/uncloud/internal/gitutil/state_test.go` — missing Git, non-repository, empty repository, clean/dirty/untracked repositories, and SHA shortening |
| Generated files | None |
| Platform-specific or build-tagged files | None |

`state.go` has only Go-standard-library imports: `fmt`, `os/exec`, `strconv`,
`strings`, and `time`. It has no internal Uncloud imports. `state_test.go` uses
the standard library plus test-only assertions from `github.com/stretchr/testify`;
that Go test dependency does not select a Rust dependency.

### Exported surface

| Symbol | Inventory obligation |
| --- | --- |
| `GitState` | Carries the commit instant (`Date`), dirty flag (`IsDirty`), repository/usable-HEAD flag (`IsRepo`), and full commit ID (`SHA`). All four exported fields are read by the direct caller or its tests. |
| `InspectGitState(dir string) (GitState, error)` | Inspects the directory by discovering and invoking the external `git` executable. |
| `(*GitState).ShortSHA(length int) string` | Returns the requested ASCII commit-ID prefix, or the full value for non-positive/oversized lengths. |

Unexported `isGitRepo` and `gitCommand` are implementation details, but their
command order, working-directory behavior, error suppression, and error text
are observable through `InspectGitState` and are therefore included below.

### Direct callers

| Caller | Use of this package |
| --- | --- |
| `upstream/uncloud/pkg/client/compose/imagetemplate.go:11,31,38,72-79,129-148` | Inspects `project.WorkingDir`; aborts image processing on an inspection error; uses `IsRepo`, `Date`, `IsDirty`, `SHA`, and `ShortSHA` to render image tags and Git-aware templates. |
| `upstream/uncloud/pkg/client/compose/imagetemplate_test.go:13,18-176,178-329` | Integration coverage calls `InspectGitState` on clean and dirty repositories and constructs `GitState` directly to verify the full SHA/date/dirty/repository contract used by image templates. |

Repository-wide exact-import and qualified-use searches find no other direct
caller. No package below `internal/gitutil` exists, so there is no nested Go
package to inventory separately.

## Behavior contract

| ID | Input or event | Required result | Errors, ordering, timing, or limitation | Evidence |
| --- | --- | --- | --- | --- |
| B01 | `InspectGitState(dir)` when `git` cannot be found through the process `PATH` | Return a non-repository state: false dirty/repository flags, empty SHA, zero/absent commit instant; return no error. | Missing Git is deliberately indistinguishable from a non-repository. Discovery happens before examining `dir`. | `state.go:23-30`; `state_test.go:13-27` |
| B02 | A directory for which `git rev-parse --git-dir` fails, including a nonexistent/unusable working directory | Return the same non-repository state and no error. | Every failure from this probe is swallowed, including process-start, permission, Git trust/configuration, and ordinary “not a repository” failures. Git searches parent directories, so a nested directory is considered part of its ancestor repository. | `state.go:32-37,69-87`; `state_test.go:29-47` |
| B03 | An initialized repository whose `git rev-parse --verify HEAD` fails (normally a repository with no commits) | Clear `IsRepo` back to false and return the otherwise empty state with no error. | Every HEAD-verification failure is swallowed as the empty-repository fallback, not only the expected unborn-HEAD error. | `state.go:39-46`; `state_test.go:49-63` |
| B04 | A usable repository with a current commit | Return `IsRepo=true`; the trimmed stdout of `git rev-parse --verify HEAD` as the full SHA; the base-10 `%ct` commit timestamp as a UTC instant at whole-second precision; and the dirty result from B05. | The code does not validate that the trimmed SHA is 40 hexadecimal characters even though that is the documented/normal result. Timestamp parsing accepts a signed 64-bit Unix-seconds value after trimming. | `state.go:39-66`; `state_test.go:65-80`; `imagetemplate_test.go:18-37` |
| B05 | `git status --porcelain` succeeds | Set `IsDirty` exactly when trimmed status stdout is non-empty. Modified tracked files and untracked files are dirty; clean output is not. | Staged changes also produce porcelain output and are dirty. Git-ignored files do not appear under the fixed command and therefore remain clean. The package inherits Git configuration and ignore rules. | `state.go:59-64`; `state_test.go:82-116`; `imagetemplate_test.go:114-149` |
| B06 | `ShortSHA(length)` on an ASCII SHA | Return the first `length` bytes when `1 <= length <= SHA byte length`; return the full SHA when `length <= 0` or exceeds its byte length; return empty for an empty SHA. | The Go operation counts bytes, not Unicode scalar values. Package-produced commit IDs and all caller-provided oracle values are ASCII; Rust need not reproduce invalid-UTF-8 Go strings or Go pointer/nil mechanics. | `state.go:90-98`; `state_test.go:118-151`; `imagetemplate.go:129-135`; `imagetemplate_test.go:182-275` |
| B07 | A normal inspection | Perform the observable operations in order: executable lookup; `git rev-parse --git-dir`; `git rev-parse --verify HEAD`; `git log -1 --format=%ct`; `git status --porcelain`. Run each command with `dir` as its working directory and inherit the process environment. | Commands are separate processes. There is no timeout, cancellation, retry, repository lock, or atomic snapshot, so a command may hang and concurrent repository changes can yield a state assembled from different moments. | `state.go:26-66,69-87` |
| B08 | The commit-timestamp command fails after a SHA was obtained, or its trimmed stdout is not a base-10 signed 64-bit integer | Return the partial state (`IsRepo=true`, SHA already populated, zero/absent date, clean/false dirty) plus an error contextualized as `get current commit timestamp: ...` or `parse current commit timestamp: ...`. | Do not downgrade these failures to non-repository and do not continue to status. Platform-specific underlying OS/parse wording may differ, but the stable operation context and partial-state boundary must remain. | `state.go:48-57` |
| B09 | The status command fails after SHA and date were obtained | Return the partial state (`IsRepo=true`, populated SHA/date, false dirty) plus an error contextualized as `check git status: ...`. | Do not downgrade to non-repository. A committed bare repository is one observable case: the earlier Git queries can succeed, while work-tree status fails. | `state.go:59-63,76-85` |
| B10 | An invoked Git command exits unsuccessfully | For surfaced B08/B09 failures, retain a stable “git command failed” category and include Git's captured stderr; non-exit process errors retain their underlying cause. | Probe/HEAD errors with the same underlying cause remain swallowed by B02/B03. Captured stderr is not trimmed and can contain its trailing newline. The compose caller wraps surfaced errors with `inspect git state at '<dir>': ...`. | `state.go:41-45,49-55,60-63,76-85`; `imagetemplate.go:71-75` |
| B11 | A clean, dirty, or non-Git state reaches the compose caller | Supply enough state for clean tags to use the UTC commit date and SHA prefix, dirty tags to add `.dirty`, and non-repository/empty-repository cases to take the caller's wall-clock fallback. | This crate does not own template formatting or the fallback clock, but must not erase or reinterpret the fields on which those observable caller results depend. | `imagetemplate.go:14-21,31,71-76,129-158`; `imagetemplate_test.go:39-175` |

There is no persistence, network access, package-owned concurrency, generated
format, or package-specific platform branch. The contract applies on each host
supported by the port where the dependency decision can provide the required
local-repository behavior. Host-native process lookup/start errors may differ in
their unstable suffix, while the stable classifications and context above do
not.

## Rust design freedom

Rust must expose an idiomatic state/result contract from which the downstream
compose port can obtain: usable-repository status, full commit ID, shortened
ASCII commit ID, UTC commit instant at second precision, dirty status, and the
partial state attached to B08/B09 failures. Exact Go struct layout, exported
field mutability, pointer receivers, helper functions, file layout, and function
names are not required.

The implementation may model the timestamp as an instant, signed Unix seconds,
or another idiomatic type so long as UTC whole-second semantics and downstream
formatting are lossless. It may introduce an internal command boundary for
deterministic tests. It must not hide B01-B10 semantic distinctions behind a
generic success/failure result. The approved dependency decision may shape the
internal design; this packet does not choose a crate, library-vs-CLI approach,
or error package.

## Dependency capabilities

| Capability | Decision record | Status |
| --- | --- | --- |
| Inspect a local Git repository with the oracle's executable discovery, ancestor-worktree/unborn-HEAD handling, commit ID and Unix timestamp retrieval, porcelain dirty-state semantics, command failure/stderr behavior, inherited environment, and supported-host behavior | `migration/dependencies/git-repository-inspection.md` | `research-required` |

No existing row in `migration/DEPENDENCIES.tsv` approves this capability. The
researcher must decide the popular, idiomatic Rust solution (including whether
the system Git executable remains a required external tool) against the full
B01-B10 hard requirements; this packet deliberately does not select a candidate.
No internal migration crate is required by this package. Rust standard-library
facilities need no dependency decision.

## Test traceability

| Behavior ID | Go test or source evidence | Required Rust test | Result |
| --- | --- | --- | --- |
| B01 | `TestInspectGitState_GitNotAvailable` | `inspect_git_missing_returns_non_repo_without_error` | `pending` |
| B02 | `TestInspectGitState_NotARepo`; `state.go:32-37,69-87` | `inspect_non_repo_and_failed_probe_are_swallowed` and `nested_directory_uses_ancestor_repo` | `pending` |
| B03 | `TestInspectGitState_EmptyRepo` | `inspect_unborn_head_returns_non_repo_without_error` | `pending` |
| B04 | `TestInspectGitState_CleanRepo`; `TestProcessImageTemplates_Integration/clean_repository` | `inspect_clean_repo_returns_sha_and_utc_commit_second` | `pending` |
| B05 | `TestInspectGitState_DirtyRepo`; `TestInspectGitState_UntrackedFiles` | `porcelain_status_covers_clean_modified_staged_untracked_and_ignored` | `pending` |
| B06 | `TestGitState_ShortSHA`; `TestGitState_ShortSHA_Empty`; `TestProcessImageTemplate` | `short_sha_ascii_length_contract` | `pending` |
| B07 | `state.go:26-87` | `inspection_uses_required_command_order_directory_and_environment` | `pending` |
| B08 | `state.go:48-57` | `timestamp_command_and_parse_failures_return_contextual_partial_state` | `pending` |
| B09 | `state.go:59-63`; bare-repository consequence of the fixed commands | `status_failure_returns_contextual_partial_state` and `committed_bare_repo_surfaces_status_failure` | `pending` |
| B10 | `state.go:76-85`; `imagetemplate.go:71-75` | `exit_stderr_and_operation_context_are_preserved` | `pending` |
| B11 | `TestProcessImageTemplates_Integration`; `TestProcessImageTemplate` | `oracle_fixture_matrix` | `pending` |

The B02, B05, and B07-B10 cases not directly asserted upstream are required
characterization tests of source-visible behavior used to distinguish fallback
from caller-fatal errors. Port test cases rather than Go helper structure.

## Acceptance commands

Run from the repository root after the integrator has registered the crate and
all dependency decisions referenced above are approved:

```sh
cargo fmt --manifest-path crates/ployz-internal-gitutil/Cargo.toml --check
cargo check --manifest-path crates/ployz-internal-gitutil/Cargo.toml --all-targets
cargo test --manifest-path crates/ployz-internal-gitutil/Cargo.toml --all-targets
cargo clippy --manifest-path crates/ployz-internal-gitutil/Cargo.toml --all-targets --all-features -- -D warnings
```

Verify the frozen oracle tree and run the exact package and direct-caller oracle
tests with the `mise.toml` toolchain documented by `UPSTREAM_ORACLE.md`:

```sh
test "$(git rev-parse HEAD:upstream/uncloud)" = a1959e967bbde8577ed4a19d367e8ee4b1ecf2bd
mise exec -C upstream/uncloud -- go test -count=1 ./internal/gitutil
mise exec -C upstream/uncloud -- go test -count=1 -run '^(TestProcessImageTemplates_Integration|TestProcessImageTemplate)$' ./pkg/client/compose
```

The targeted differential gate pairs the complete Go fixture matrix with the
required Rust integration test. The Rust test must exercise equivalent missing
Git, non-repository, unborn-HEAD, clean, modified, staged, untracked, ignored,
nested-directory, and committed-bare-repository fixtures, and must assert the
same stable state/error properties rather than fixed generated SHAs or commit
times:

```sh
mise exec -C upstream/uncloud -- go test -count=1 -run '^(TestInspectGitState_(GitNotAvailable|NotARepo|EmptyRepo|CleanRepo|DirtyRepo|UntrackedFiles)|TestGitState_(ShortSHA|ShortSHA_Empty))$' ./internal/gitutil
cargo test --manifest-path crates/ployz-internal-gitutil/Cargo.toml --test oracle_differential -- --exact oracle_fixture_matrix
```

These tests require a usable local `git` executable for all fixtures except the
explicit missing-Git case. They require no network, Docker daemon, privileges,
or external repository. There is no platform-specific acceptance exception.

## Handoff

The implementor records its commit, deliberate behavior mappings, and check
results here. Reviewers record findings using `migration/REVIEW_TEMPLATE.md`.
The controller records state changes and blockers.

- Implementor commit: pending
- Deliberate behavior mappings: pending
- Targeted check results: pending
- Parity review: pending
- Rust review: pending
- Blockers: dependency decision `migration/dependencies/git-repository-inspection.md` is not yet present or approved; no behavior question is unresolved.
