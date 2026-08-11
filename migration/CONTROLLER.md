# Controller workflow

The controller designs and schedules the port. It does not implement crates or
review its own packets.

## Build the package graph

Catalog every Go package from the oracle. Record one row per package in
`migration/PACKAGES.tsv`, including generated and platform-specific packages.
Derive dependency edges from Go imports and verify nested directories as
separate packages.

Completion criterion: every in-scope Go package appears exactly once and every
internal import resolves to another row.

## Prepare packets

Dispatch packetizer agents in parallel. Each packetizer reads the oracle and
owns one distinct path under `migration/packages/`; it does not implement code.
Packetizers use `migration/PACKAGE_TEMPLATE.md`. The controller checks each
packet against all direct callers and tests before changing its state to
`contract-ready`.

Group identical dependency capabilities across packets. Create one dependency
request per capability and dispatch one fresh research agent. Apply approved
decisions to every waiting packet.

Completion criterion: a `ready` packet contains a closed behavior contract,
complete initial traceability, and approved dependency decisions.

## Schedule a wave

A wave contains only `ready` packages whose internal dependencies are already
integrated or included earlier in the same ordered wave. Freeze public crate
contracts needed by concurrent consumers before dispatch.

Give every implementor:

- one package packet;
- one exclusive crate directory;
- one isolated branch or worktree;
- the exact base commit;
- the targeted acceptance commands.

The controller owns no implementation branch. Shared workspace files remain
with the integrator.

An implementor dispatch needs no conversation history. Use this task prompt:

```text
Implement only the ready package packet at <packet path>. Follow AGENTS.md and
PORTING.md. Change only the owned crate path. Return the crate commit and
targeted acceptance results to the controller.
```

If the packet is not in `ready`, do not dispatch an implementor.

## Advance state

Require a concrete artifact for every transition:

| Transition | Required artifact |
| --- | --- |
| `catalogued -> contract-ready` | complete package packet |
| `contract-ready -> ready` | all dependency decisions approved |
| `ready -> implementing` | exclusive owner and base commit assigned |
| `implementing -> reviewing` | green targeted checks and crate commit |
| `reviewing -> fixing` | actionable review findings |
| `reviewing -> accepted` | two clean fresh reviews |
| `accepted -> integrated` | green workspace and oracle checks |

Never advance state from an agent's confidence statement alone.

## Recover failures

- A dependency request returns the package to `dependency-blocked`.
- Contract churn invalidates every dependent ready packet; update those packets
  before dispatch.
- A merge or workspace failure returns the owning package to `fixing`.
- An unavailable platform or privileged test keeps the package blocked unless a
  human records an explicit verification exception.
