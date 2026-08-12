# Dependency decision: `embedded-read-only-sqlite-access`

| Field | Value |
| --- | --- |
| Status | **`approved`** by explicit user authority on 2026-08-12 for ordinary logical-read-only behavior and the documented storage limitations |
| Capability | Embedded read-only inspection of an existing Corrosion `store.db` |
| Strongest conditional candidate | `rusqlite = { version = "=0.40.2", default-features = false, features = ["bundled", "limits"] }` |
| Embedded engine | `libsqlite3-sys = 0.38.2`; SQLite `3.53.2` |
| License | Rusqlite and `libsqlite3-sys`: `MIT`; SQLite: public domain |
| Research date | `2026-08-12` UTC |
| Request | Direct user assignment; no request file exists at this base |
| Integration base | `ceb86fd1d34e9be60bc994307fedb7b1ef63611b` |
| Affected package | `internal/machine/corromigrate` |

## Decision

Approve `rusqlite` 0.40.2 with `bundled,limits` under the user's explicit
2026-08-12 dependency-unblock authority. It is the strongest popular,
maintained, idiomatic Rust dependency for the queries and typed row handling,
but SQLite itself cannot satisfy this assignment's simultaneous requirements
for an **arbitrary** existing database:

1. recover every valid state, including committed data present only in a WAL;
2. create no database, journal, WAL, shared-memory, or other on-disk file; and
3. mutate no existing database or auxiliary-file content, size, or directory
   entry. Filesystem-managed access timestamps are outside what an embedded
   database library can guarantee.

`SQLITE_OPEN_READONLY` protects the main database and refuses to create a
missing main file, but a WAL-mode reader may create `-wal`/`-shm` files and
uses the `-shm` wal-index for coordination. SQLite documents that a read-only
WAL database requires existing readable sidecars, permission to create them,
or an immutable connection
([open flags](https://www.sqlite.org/c3ref/open.html),
[read-only WAL](https://www.sqlite.org/wal.html#read_only_databases),
[WAL-index lifecycle](https://www.sqlite.org/walformat.html)). The exact Rust
1.96 probe asserted both effects: a quiescent WAL database with only `store.db`
gained new sidecars (a zero-byte WAL and 32 KiB SHM in this run), while reading
a live WAL changed existing `-shm` bytes.

The apparent no-write alternative, URI `immutable=1`, disables locking and
change detection because it assumes the database cannot change
([URI contract](https://www.sqlite.org/uri.html#recognized_query_parameters)).
It left all files byte-identical in the probe, but did not replay a committed,
uncheckpointed WAL: the table committed only in that WAL was reported as
missing. `PRAGMA locking_mode=EXCLUSIVE` on an ordinary read-only connection
also did not provide a loophole; both quiescent and crash-left WAL fixtures
failed with extended code 3850 (`SQLITE_IOERR_LOCK`). The built-in `unix-excl`
VFS did read a crash-left WAL with its heap wal-index, but still changed the
on-disk auxiliary-file set/content. An ordinary read-only query against a hot
rollback journal failed with `SQLITE_READONLY_ROLLBACK`; a writable control
recovered it. Copying or checkpointing
the database would itself create or mutate storage and is outside this
capability.

This is an SQLite storage-model constraint, not a rusqlite deficiency. SQLx,
Diesel, the `sqlite` crate, and direct `libsqlite3-sys` all use the same native
engine and inherit it. A production overlay VFS would have to snapshot and
coordinate the main file, WAL, SHM, and rollback journal across concurrent
processes, hostile sizes, unsafe callbacks, and all four target combinations.
No maintained production Rust crate meeting those gates was found; building
one is a storage subsystem, not a narrow dependency seam.

The user selected this contract option on 2026-08-12:

- permit SQLite's logical-read-only semantics and possible `-wal`/`-shm`
  creation/update.

This approval permits SQLite logical-read-only semantics and possible WAL/SHM
creation or mutation. The implementation must retain the other bounded safety
and lifecycle constraints in this record.

## Oracle, tests, callers, and explicit contract

The source/caller search covered `upstream/uncloud/**` except the explicitly
out-of-scope `experiment/**` tree.

- [`migrate.go`](../../upstream/uncloud/internal/machine/corromigrate/migrate.go)
  opens `file:<path>?mode=ro`; checks exact `type='table'` and
  `name='__corro_bookkeeping'`; then iterates `SELECT key, value FROM cluster`
  and `SELECT id, info FROM machines` without `ORDER BY`. It separately wraps
  open, query, scan, and post-iteration errors.
- [`migrate_test.go`](../../upstream/uncloud/internal/machine/corromigrate/migrate_test.go)
  covers exact old/new bookkeeping names, empty databases, and multiple text
  rows. It does not cover WAL sidecars, malformed files, dynamic storage
  classes, path metacharacters, or cancellation.
- [`machine.go`](../../upstream/uncloud/internal/machine/machine.go) invokes
  migration during startup after a best-effort legacy-service stop and before
  replacement Corrosion starts. A stop failure is swallowed, so the oracle
  does not prove the files are immutable or fully checkpointed.
- [`cluster.go`](../../upstream/uncloud/internal/machine/cluster.go) later
  applies the dumped seed. Missing a committed WAL row is therefore an
  observable data-loss defect, not an internal implementation choice.

### Assigned TEXT behavior differs from the Go implementation

The assignment explicitly requires incompatible/non-TEXT values to be
rejected. Rusqlite naturally does that: `Row::get::<_, String>` accepts only
SQLite `TEXT`, rejects INTEGER/REAL/BLOB/NULL as `InvalidColumnType`, and
rejects invalid UTF-8
([`String` conversion](https://github.com/rusqlite/rusqlite/blob/v0.40.2/src/types/from_sql.rs#L201-L206),
[`ValueRef::as_str`](https://github.com/rusqlite/rusqlite/blob/v0.40.2/src/types/value_ref.rs#L81-L91),
[`Row::get`](https://docs.rs/rusqlite/0.40.2/rusqlite/struct.Row.html#method.get)).

The frozen Go implementation is broader: `database/sql` converts INTEGER and
REAL values to decimal strings and BLOB bytes directly to a Go string; only
NULL fails, and invalid UTF-8 bytes survive in a Go string
([Go conversion source](https://go.dev/src/database/sql/convert.go)). A probe
against the oracle's exact `modernc.org/sqlite v1.36.3` observed:

```text
INTEGER 42 -> "42"              success
REAL 3.5 -> "3.5"               success
BLOB x'6162' -> "ab"            success
NULL -> conversion error
CAST(x'80' AS TEXT) -> byte 0x80 success
```

This record treats the user's explicit TEXT-only requirement as authoritative,
but implementors and reviewers must not claim that it is byte-for-byte Go
scan parity.

## Hard gates

| Gate | Requirement and evidence | Result |
| --- | --- | --- |
| Read-only/no-create/no-mutation | Missing main path must fail; all SQL writes must fail; every valid rollback/WAL state must remain byte-identical. Ordinary read-only passed for the main file but created/updated WAL sidecars. Immutable preserved bytes but missed committed WAL-only state. Exclusive read-only returned `SQLITE_IOERR_LOCK`. | **`fail`** |
| Queries, rows, and types | `open_with_flags`, fixed SQL, fallible `Rows::next`, and strict `String` extraction cover exact table detection, delivered row order, query/scan/iteration errors, and assigned TEXT-only semantics. | `pass` |
| Malformed/hostile files | SQLite promises validation/error rather than crashes for malformed input and recommends defensive configuration and lowered limits. Random bytes, truncated headers, invalid page size, invalid UTF-8, dynamic-type fixtures, and a schema exceeding the parser-depth limit returned errors without panic in the probe. | `pass` for bounded probe; native C residual risk remains |
| Cancellation and errors | `InterruptHandle` is `Send + Sync`; `sqlite3_interrupt` produced `SQLITE_INTERRUPT` during an expensive recursive query. Package checks are still required before open/query and between rows, with distinct contextual labels. | `pass` |
| License/security | Direct and resolved Rust licenses are permissive; SQLite is public domain. The exact 12-package lock audited clean against the 1,216-entry RustSec database. Bundled SQLite 3.53.2 is recent but trails the official 3.53.3 patch release. No applicable published advisory was found, but the lag must be rechecked after the contract blocker is resolved. | `pass` for the conditional candidate, with update obligation |
| Maintenance/adoption | Rusqlite 0.40.2 was released 2026-08-08; its repository is active and unarchived, and its [official crates.io page](https://crates.io/crates/rusqlite) shows broad use. | `pass` |
| Rust/platforms | Release MSRV is 1.88, below 1.96. Exact-feature checks passed on Linux x86_64 and cross-compiled Linux aarch64. Upstream CI tests bundled x86_64 Linux/macOS. Native macOS x86_64/aarch64 was unavailable in this Linux VM. | `pass` at source/support gate; native macOS acceptance still required |
| Architecture | In-process synchronous library; no executable, service, daemon, network, or runtime system SQLite. Bundled build needs a C compiler only at build time. | `pass` |

Primary maintenance/toolchain sources:

- Rusqlite's [0.40.2 release](https://github.com/rusqlite/rusqlite/releases/tag/v0.40.2)
  records the MSRV reduction to Rust 1.88.
- The tagged [README](https://github.com/rusqlite/rusqlite/blob/v0.40.2/README.md)
  documents `bundled`, `limits`, the embedded SQLite 3.53.2 engine, build-time
  `cc` behavior, and licenses.
- The tagged [CI matrix](https://github.com/rusqlite/rusqlite/blob/v0.40.2/.github/workflows/main.yml)
  runs bundled builds/tests on Linux and macOS, plus ASan, Miri, Clippy, and
  minimum-version jobs.
- SQLite's [3.53.2 release record](https://sqlite.org/releaselog/3_53_2.html)
  gives the embedded source ID and patch date, while the official
  [release history](https://sqlite.org/changes.html) records the newer 3.53.3
  patch; its
  [security guidance](https://www.sqlite.org/security.html) describes hostile
  database handling, defensive mode, trusted schemas, and runtime limits.

## Candidate comparison

| Candidate | Evidence and result |
| --- | --- |
| **Rusqlite 0.40.2, `bundled,limits`** | Strongest conditional candidate. Direct synchronous SQLite API, explicit flags, strict types, fallible row stepping, interruption, defensive configuration, current embedded SQLite, narrow dependency graph, and dominant adoption. Fails only the unavoidable arbitrary-WAL/no-file-effects gate. |
| `sqlx 0.9.0`, SQLite | Maintained and popular with Rust 1.94 support, but adds an async worker/channel, executor, URL, pooling, and broader SQL toolkit for three synchronous startup queries. It uses SQLite and has the same WAL constraint. [Manifest](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-sqlite/Cargo.toml). |
| `diesel 2.3.12`, SQLite | Maintained ORM/query-builder with unnecessary schema/derive machinery; its ordinary establishment path may create a missing database and still inherits SQLite WAL semantics. [Connection API](https://docs.rs/diesel/2.3.12/diesel/sqlite/struct.SqliteConnection.html). |
| `sqlite 0.37.0`, `bundled` | Smaller flag-aware wrapper whose published release embeds an older engine. It provides no escape from SQLite's WAL model. [Published crate](https://docs.rs/crate/sqlite/0.37.0). |
| Direct `libsqlite3-sys` | Same engine behavior with package-owned unsafe FFI, error, row-lifetime, and cancellation work. Rejected as less safe and less idiomatic. |
| Built-in or custom overlay VFS | SQLite documents that `unix-excl`/exclusive locking keeps the WAL index in heap, but the probe showed `unix-excl` still changed crash-WAL files. [`sqlite-vfs` 0.2.0](https://docs.rs/crate/sqlite-vfs/0.2.0) is explicitly a non-production, unaudited prototype; it lacks WAL and memory mapping and has UNIX-only tests. A package-owned FFI VFS or RAM snapshot/WAL overlay fails maintenance, safety, race-consistency, bounded-resource, and platform gates. [SQLite VFS](https://sqlite.org/vfs.html), [WAL-index variations](https://sqlite.org/walformat.html#variations). |
| Turso pure-Rust reimplementation | Its [compatibility matrix](https://github.com/tursodatabase/turso/blob/main/COMPAT.md) documents partial SQLite compatibility, unsupported rollback-journal modes, and unsupported multi-process access. It cannot accept arbitrary existing SQLite states at this seam. |
| Pure-Rust/local parser, installed CLI, or external service | No mature maintained parser was found that can safely recover arbitrary SQLite/WAL files with required compatibility. A local parser is migration-scale reimplementation; CLI/service options are expressly forbidden. |

## Conditional integration if the hard gate is narrowed

The exact dependency would be:

```toml
rusqlite = { version = "=0.40.2", default-features = false, features = ["bundled", "limits"] }
```

`bundled` statically builds the published SQLite 3.53.2 amalgamation and avoids
host-specific library availability, compile options, versions, and patch
levels. The tradeoffs are build time, binary size, a build-time C compiler, and
the obligation to update the crate promptly for SQLite fixes. A system SQLite
is rejected for this migration because results and security posture would vary
by host. The `limits` feature adds only rusqlite API surface and permits bounded
hostile-input configuration; it adds no dependency.

For ordinary logical-read-only behavior, use only
`SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`; do not use rusqlite's default
read-write/create/URI flags. Pass the filesystem `Path` directly so `?`, `#`,
`%`, spaces, and non-UTF-8 bytes cannot become URI controls. Apply, before any
schema query:

- `SQLITE_DBCONFIG_DEFENSIVE = true`;
- `SQLITE_DBCONFIG_TRUSTED_SCHEMA = false`;
- `PRAGMA query_only = ON` so accidental TEMP writes also fail;
- conservative per-connection limits, at minimum 64 MiB `LENGTH`, 100,000-byte
  `SQL_LENGTH`, 100 `COLUMN`, 100 `EXPR_DEPTH`, 10 `COMPOUND_SELECT`, 100,000
  `VDBE_OP`, 100 `PARSER_DEPTH`, zero `ATTACHED`, and zero worker threads.

Those limits intentionally reject oversized or structurally hostile databases
instead of exhausting process resources. They are a conditional accepted
limitation, not Go parity. Do not run an unconditional whole-database
`quick_check`: it can turn unrelated corruption or database size into a new
startup failure and denial-of-service surface. The three fixed queries already
surface pages/schema they actually touch.

Use the oracle SQL without `ORDER BY`, append each row in SQLite's delivered
sequence, and use a manual `Rows::next` loop so these package contexts remain
distinct:

```text
open/configure store.db
detect old store version
query cluster
scan cluster row
iterate cluster rows
query machines
scan machine row
iterate machine rows
```

Check cancellation before open, before each prepare/query, and between rows.
For a blocked `prepare`/`step`, a package-owned joined guard may call
`Connection::get_interrupt_handle().interrupt()` from the cancellation side.
Join/drop that guard before the connection closes. SQLite interruption is
cooperative and an interrupt issued while no statement is active is a no-op
([rusqlite handle](https://docs.rs/rusqlite/0.40.2/rusqlite/struct.InterruptHandle.html),
[SQLite interruption](https://www.sqlite.org/c3ref/interrupt.html)).

## License and security notes

The generated exact-feature lock contained 12 packages. All reported
`MIT`, `Apache-2.0`, or dual permissive licenses; Rusqlite and
`libsqlite3-sys` are MIT, and bundled SQLite is public domain
([tagged license](https://github.com/rusqlite/rusqlite/blob/v0.40.2/LICENSE),
[SQLite copyright](https://sqlite.org/copyright.html)). `cargo audit` found no
known RustSec vulnerability or warning. The historical `libsqlite3-sys`
uninitialized-memory advisory is fixed since 0.25.1; selected 0.38.2 is newer
([RUSTSEC-2022-0090](https://rustsec.org/advisories/RUSTSEC-2022-0090.html)).

Bundling native C does not eliminate future SQLite vulnerabilities. SQLite
states that malformed database files should produce errors rather than crashes
and fuzzes this boundary, but zero-day memory-safety risk remains. Keep loadable
extensions and custom SQL functions disabled, lower limits, retain
`trusted_schema = false`, and update the exact dependency when SQLite ships
security fixes. The workspace must resolve one `libsqlite3-sys` linkage;
multiple independent SQLite copies can defeat locking coordination
([SQLite warning](https://sqlite.org/howtocorrupt.html#multiple_copies_of_sqlite_linked_into_the_same_application)).

## Probe and checks

The runnable probe is outside the repository at
`/tmp/ployz-sqlite-probe-20260812`; it is deliberately not a committed
artifact. It pins Rust 2024 edition, rusqlite 0.40.2, `default-features=false`,
and `bundled,limits`. It covered:

- missing-file no-create; filesystem names containing spaces, `?`, `#`, `%`;
- exact bookkeeping table versus view, near name, and case variant;
- row sequence from `cluster` and `machines` without `ORDER BY`;
- strict INTEGER, REAL, NULL, BLOB, and invalid-UTF-8 rejection;
- random, truncated, header-only, and invalid-page-size database inputs without
  panic;
- read-only rejection of main writes and TEMP-table creation;
- `SQLITE_INTERRUPT` from another thread during a long recursive query;
- asserted content/size/directory-entry snapshots for clean and hot rollback,
  live WAL, cleanly closed WAL, crash-left WAL, immutable, exclusive-locking,
  and `unix-excl` modes; filesystem-managed atime was not asserted.

Observed storage matrix:

| Fixture/open mode | Query result | File effect |
| --- | --- | --- |
| Rollback database, ordinary read-only | correct | main file/directory byte-identical |
| Missing file, ordinary read-only | error | no file created |
| Live WAL, ordinary read-only | sees committed WAL row | existing `-shm` bytes changed |
| Cleanly closed WAL with no sidecars, ordinary read-only | sees row | creates `-wal` and `-shm` |
| Cleanly closed WAL, `immutable=1` | sees checkpointed row | byte-identical |
| Crash-left committed WAL, `immutable=1` | `no such table` | byte-identical but incorrect/incomplete |
| Quiescent or crash-left WAL, read-only exclusive locking | `SQLITE_IOERR_LOCK` | byte-identical but unusable |
| Crash-left WAL, read-only `unix-excl` VFS | sees committed WAL row | changes auxiliary files |
| Crash-left hot rollback journal, ordinary read-only | `SQLITE_READONLY_ROLLBACK` | byte-identical but unusable; writable control recovers it |

An ordinary read-only control connection against a separately generated but
identical crash-left fixture did see `"crash-wal"`, confirming that the
immutable failure was omission of valid committed WAL data rather than an
invalid fixture.

Commands and results:

```sh
cargo run --locked --manifest-path /tmp/ployz-sqlite-probe-20260812/Cargo.toml
# PASS; rusqlite reported SQLite 3.53.2 / 3053002

cargo fmt --check --manifest-path /tmp/ployz-sqlite-probe-20260812/Cargo.toml
cargo clippy --locked --all-targets --all-features \
  --manifest-path /tmp/ployz-sqlite-probe-20260812/Cargo.toml -- -D warnings
cargo audit --file /tmp/ployz-sqlite-probe-20260812/Cargo.lock
cargo check --locked --target x86_64-unknown-linux-gnu \
  --manifest-path /tmp/ployz-sqlite-probe-20260812/Cargo.toml
cargo check --locked --target aarch64-unknown-linux-gnu \
  --manifest-path /tmp/ployz-sqlite-probe-20260812/Cargo.toml
# all pass on Rust 1.96.0; audit loaded 1,216 advisories and reported none
```

macOS target checks from Linux reached the bundled C build and failed because
the VM has neither a macOS SDK nor an Apple-target C compiler (`cc` rejected
`-arch` and `-mmacosx-version-min`), not because of Rust source incompatibility.
Native x86_64 and aarch64 macOS checks remain required after any contract
resolution. The frozen Go package test could not run because its declared Go
1.26 toolchain was unavailable; the smaller exact-driver Go scan probe ran
under the downloaded Go 1.25.12 toolchain.

## Accepted limitations

Approved limitations are WAL/SHM creation and mutation, cooperative
cancellation, strict TEXT behavior differing from Go coercion, bounded
hostile-input limits, native C residual risk, bundled build cost, and
unspecified row ordering across engine versions.

## Review

The initial fresh read-only adversarial review found seven issues: mislabeled
immutable probe paths; decisive WAL effects not asserted; inaccurate SHM size
wording; missing VFS-overlay and hot-rollback analysis; missing parser-depth
coverage; untraceable precise adoption counts; and an overbroad meaning of file
mutation. All were corrected. The probe now separately asserts ordinary and
immutable modes, WAL effects, `unix-excl`, hot-journal recovery, and hostile
parser depth; the decision and limitations now match that evidence.

After those corrections, a different fresh read-only reviewer rechecked the
decision, sources, oracle compatibility, candidate coverage, and runnable probe
and returned exactly `CLEAN` with no actionable findings on 2026-08-12 UTC.
